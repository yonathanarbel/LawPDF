//! Bounded parsing of PDF link destinations and the document outline.
use super::*;
use crate::model::{PdfNavigation, PdfOutlineItem};
use std::cell::RefCell;
use std::collections::HashSet;

const MAX_TREE_NODES: usize = 10_000;
const MAX_DEPTH: usize = 32;
const MAX_LINKS_PER_PAGE: usize = 10_000;
const MAX_LINKS_PER_DOCUMENT: usize = 100_000;

pub(super) fn load_pdf_navigation(source: &Path, page_count: usize) -> Result<PdfNavigation> {
    let document = Document::load(source)
        .with_context(|| format!("failed to load source PDF {}", source.display()))?;
    navigation_for_document(&document, page_count)
}

fn navigation_for_document(document: &Document, page_count: usize) -> Result<PdfNavigation> {
    let pages = document.get_pages();
    let destinations = named_destinations(document);
    let page_ids: HashMap<ObjectId, usize> = pages
        .iter()
        .filter_map(|(number, id)| {
            let index = number.saturating_sub(1) as usize;
            (index < page_count).then_some((*id, index))
        })
        .collect();
    let page_objects = page_ids.iter().map(|(id, page)| (*page, *id)).collect();
    let resolver = DestinationResolver {
        document,
        pages: page_ids,
        page_objects,
        names: destinations,
        geometries: RefCell::new(HashMap::new()),
    };
    let mut links = vec![Vec::new(); page_count];
    let mut examined_annotations = 0usize;
    for (number, page_id) in pages {
        let page_index = number.saturating_sub(1) as usize;
        if page_index >= page_count {
            continue;
        }
        let page = document.get_object(page_id)?.as_dict()?;
        let Some(annots) = page
            .get(b"Annots")
            .ok()
            .and_then(|object| dereference(document, object))
        else {
            continue;
        };
        let items: &[Object] = match annots {
            Object::Array(items) => items,
            item => std::slice::from_ref(item),
        };
        for annotation in items.iter().take(MAX_LINKS_PER_PAGE) {
            if examined_annotations >= MAX_LINKS_PER_DOCUMENT {
                break;
            }
            examined_annotations += 1;
            if let Some(link) = parse_link(&resolver, annotation) {
                links[page_index].push(link);
            }
        }
        let geometry = pdf_page_geometry(document, page_id)?;
        for link in &mut links[page_index] {
            link.rect = geometry.rect(link.rect, false);
        }
    }
    Ok(PdfNavigation {
        links,
        outline: parse_outline(&resolver),
    })
}

fn dereference<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Object> {
    let mut current = object;
    let mut seen = HashSet::new();
    for _ in 0..MAX_DEPTH {
        match current {
            Object::Reference(id) if seen.insert(*id) => current = document.get_object(*id).ok()?,
            Object::Reference(_) => return None,
            _ => return Some(current),
        }
    }
    None
}

fn name_bytes(object: &Object) -> Option<&[u8]> {
    match object {
        Object::Name(bytes) | Object::String(bytes, _) => Some(bytes),
        _ => None,
    }
}

fn named_destinations(document: &Document) -> HashMap<Vec<u8>, &Object> {
    let mut found = HashMap::new();
    let Ok(catalog) = document.catalog() else {
        return found;
    };
    if let Some(legacy) = catalog
        .get(b"Dests")
        .ok()
        .and_then(|object| dereference(document, object))
        .and_then(|object| object.as_dict().ok())
    {
        for (name, destination) in legacy.iter().take(MAX_TREE_NODES) {
            if name.len() <= 4096 {
                found.insert(name.clone(), destination);
            }
        }
    }
    let Some(root) = catalog
        .get(b"Names")
        .ok()
        .and_then(|object| dereference(document, object))
        .and_then(|object| object.as_dict().ok())
        .and_then(|names| names.get(b"Dests").ok())
    else {
        return found;
    };
    let mut stack = vec![(root, 0usize)];
    let mut visited = HashSet::new();
    let mut count = 0usize;
    let mut examined_names = 0usize;
    while let Some((node, depth)) = stack.pop() {
        if count >= MAX_TREE_NODES {
            break;
        }
        if depth > MAX_DEPTH {
            continue;
        }
        count += 1;
        if let Object::Reference(id) = node
            && !visited.insert(*id)
        {
            continue;
        }
        let Some(dict) = dereference(document, node).and_then(|object| object.as_dict().ok())
        else {
            continue;
        };
        if let Some(items) = dict
            .get(b"Names")
            .ok()
            .and_then(|object| dereference(document, object))
            .and_then(|object| object.as_array().ok())
        {
            for pair in items.chunks_exact(2).take(MAX_TREE_NODES) {
                if examined_names >= MAX_TREE_NODES {
                    return found;
                }
                examined_names += 1;
                if let Some(name) = name_bytes(&pair[0]).filter(|name| name.len() <= 4096) {
                    found.entry(name.to_vec()).or_insert(&pair[1]);
                }
            }
        }
        if let Some(children) = dict
            .get(b"Kids")
            .ok()
            .and_then(|object| dereference(document, object))
            .and_then(|object| object.as_array().ok())
        {
            let room = MAX_TREE_NODES.saturating_sub(count.saturating_add(stack.len()));
            stack.extend(
                children
                    .iter()
                    .take(room)
                    .rev()
                    .map(|child| (child, depth + 1)),
            );
        }
    }
    found
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ResolvedDestination {
    page: usize,
    fraction: Option<f32>,
}

struct DestinationResolver<'a> {
    document: &'a Document,
    pages: HashMap<ObjectId, usize>,
    page_objects: HashMap<usize, ObjectId>,
    names: HashMap<Vec<u8>, &'a Object>,
    geometries: RefCell<HashMap<usize, Option<crate::page_geometry::PageGeometry>>>,
}

impl DestinationResolver<'_> {
    fn geometry(&self, page: usize) -> Option<crate::page_geometry::PageGeometry> {
        if let Some(cached) = self.geometries.borrow().get(&page).copied() {
            return cached;
        }
        let geometry = self
            .page_objects
            .get(&page)
            .and_then(|id| pdf_page_geometry(self.document, *id).ok());
        self.geometries.borrow_mut().insert(page, geometry);
        geometry
    }

    fn explicit_fraction(&self, page: usize, items: &[Object]) -> Option<f32> {
        let kind = items
            .get(1)
            .and_then(|object| dereference(self.document, object))
            .and_then(pdf_object_text)?
            .to_ascii_uppercase();
        let geometry = self.geometry(page)?;
        let rotation = geometry.rotation.rem_euclid(360);
        let number = |index: usize| {
            items
                .get(index)
                .and_then(|object| dereference(self.document, object))
                .and_then(pdf_number)
                .filter(|n| n.is_finite())
        };
        let display_top = match kind.as_str() {
            "XYZ" => {
                let x = if matches!(rotation, 90 | 270) {
                    number(2)?
                } else {
                    number(2).unwrap_or(geometry.bounds.left)
                };
                let y = if matches!(rotation, 0 | 180) {
                    number(3)?
                } else {
                    number(3).unwrap_or(geometry.bounds.top)
                };
                geometry.to_display((x, y)).1
            }
            "FITH" | "FITBH" if matches!(rotation, 0 | 180) => {
                geometry.to_display((geometry.bounds.left, number(2)?)).1
            }
            "FITV" | "FITBV" if matches!(rotation, 90 | 270) => {
                geometry.to_display((number(2)?, geometry.bounds.top)).1
            }
            "FITR" => {
                geometry
                    .rect(
                        PdfRect::new(number(2)?, number(3)?, number(4)?, number(5)?),
                        false,
                    )
                    .top
            }
            _ => return None,
        };
        let height = if matches!(rotation, 90 | 270) {
            geometry.bounds.width()
        } else {
            geometry.bounds.height()
        };
        (height.is_finite() && height > 0.0 && display_top.is_finite())
            .then(|| (1.0 - display_top / height).clamp(0.0, 1.0))
    }

    fn resolve(&self, object: &Object, depth: usize) -> Option<ResolvedDestination> {
        if depth >= MAX_DEPTH {
            return None;
        }
        match object {
            Object::Reference(id) => self
                .pages
                .get(id)
                .copied()
                .map(|page| ResolvedDestination {
                    page,
                    fraction: None,
                })
                .or_else(|| {
                    self.document
                        .get_object(*id)
                        .ok()
                        .and_then(|next| self.resolve(next, depth + 1))
                }),
            Object::Array(items) => {
                let base = self.resolve(items.first()?, depth + 1)?;
                Some(ResolvedDestination {
                    page: base.page,
                    fraction: self.explicit_fraction(base.page, items).or(base.fraction),
                })
            }
            Object::Dictionary(dict) => dict
                .get(b"D")
                .ok()
                .and_then(|dest| self.resolve(dest, depth + 1)),
            Object::Name(_) | Object::String(_, _) => name_bytes(object)
                .and_then(|name| self.names.get(name).copied())
                .and_then(|dest| self.resolve(dest, depth + 1)),
            Object::Integer(index) if *index >= 0 => usize::try_from(*index)
                .ok()
                .filter(|index| self.page_objects.contains_key(index))
                .map(|page| ResolvedDestination {
                    page,
                    fraction: None,
                }),
            _ => None,
        }
    }

    fn from_dictionary(&self, dict: &Dictionary) -> Option<ResolvedDestination> {
        if let Some(destination) = dict
            .get(b"Dest")
            .ok()
            .and_then(|dest| self.resolve(dest, 0))
        {
            return Some(destination);
        }
        let action = dict
            .get(b"A")
            .ok()
            .and_then(|object| dereference(self.document, object))?
            .as_dict()
            .ok()?;
        if !action
            .get(b"S")
            .ok()
            .and_then(pdf_object_text)
            .is_some_and(|kind| kind.eq_ignore_ascii_case("GoTo"))
        {
            return None;
        }
        action.get(b"D").ok().and_then(|dest| self.resolve(dest, 0))
    }
}

fn parse_link(resolver: &DestinationResolver<'_>, annotation: &Object) -> Option<PageLink> {
    let dict = annotation_dict(resolver.document, annotation)?;
    if !dict
        .get(b"Subtype")
        .ok()
        .and_then(pdf_object_text)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("Link"))
    {
        return None;
    }
    let rect = dict.get(b"Rect").ok().and_then(pdf_rect_from_object)?;
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }
    if let Some(destination) = resolver.from_dictionary(dict) {
        return Some(PageLink {
            rect,
            url: String::new(),
            destination_page: Some(destination.page),
            destination_fraction: destination.fraction,
        });
    }
    let uri = dict
        .get(b"A")
        .ok()
        .and_then(|action| pdf_uri_from_action(resolver.document, action))?;
    Some(PageLink {
        rect,
        url: normalize_web_url(&uri)?,
        destination_page: None,
        destination_fraction: None,
    })
}

fn parse_outline(resolver: &DestinationResolver<'_>) -> Vec<PdfOutlineItem> {
    let document = resolver.document;
    let Some(first) = document
        .catalog()
        .ok()
        .and_then(|catalog| catalog.get(b"Outlines").ok())
        .and_then(|outline| dereference(document, outline))
        .and_then(|outline| outline.as_dict().ok())
        .and_then(|outline| outline.get(b"First").ok())
    else {
        return Vec::new();
    };
    let mut stack = vec![(first, 0usize)];
    let mut visited = HashSet::new();
    let mut result = Vec::new();
    let mut count = 0usize;
    while let Some((node, depth)) = stack.pop() {
        if depth > MAX_DEPTH || count >= MAX_TREE_NODES {
            continue;
        }
        count += 1;
        if let Object::Reference(id) = node
            && !visited.insert(*id)
        {
            continue;
        }
        let Some(dict) = dereference(document, node).and_then(|object| object.as_dict().ok())
        else {
            continue;
        };
        if let Ok(next) = dict.get(b"Next") {
            stack.push((next, depth));
        }
        if let Ok(child) = dict.get(b"First") {
            stack.push((child, depth + 1));
        }
        let Some(title) = dict.get(b"Title").ok().and_then(pdf_object_text) else {
            continue;
        };
        let title = title.trim().chars().take(1024).collect::<String>();
        if !title.is_empty() {
            let destination = resolver.from_dictionary(dict);
            result.push(PdfOutlineItem {
                title,
                page_index: destination.map(|destination| destination.page),
                page_fraction: destination.and_then(|destination| destination.fraction),
                level: depth,
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> (Document, ObjectId, ObjectId, ObjectId) {
        let mut doc = Document::with_version("1.5");
        let pages = doc.new_object_id();
        let first = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()] });
        let second = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()] });
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Count" => 2,
            "Kids" => vec![Object::Reference(first), Object::Reference(second)] }),
        );
        let root = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", root);
        (doc, root, first, second)
    }

    fn destination(page: ObjectId) -> Object {
        Object::Array(vec![Object::Reference(page), Object::Name(b"Fit".to_vec())])
    }

    fn link(dest: Object) -> Object {
        Object::Dictionary(dictionary! { "Type" => "Annot", "Subtype" => "Link",
        "Rect" => vec![10.into(), 20.into(), 100.into(), 40.into()], "Dest" => dest })
    }

    #[test]
    fn parses_direct_and_named_links_and_nested_outline() {
        let (mut doc, root, first, second) = document();
        let names = doc.add_object(
            dictionary! { "Names" => vec![Object::string_literal("chapter"), destination(second)] },
        );
        let top = doc.new_object_id();
        let child = doc.new_object_id();
        doc.objects.insert(top, Object::Dictionary(dictionary! {
            "Title" => Object::string_literal("Chapter"), "Dest" => Object::Name(b"chapter".to_vec()),
            "First" => child, "Next" => top,
        }));
        doc.objects.insert(child, Object::Dictionary(dictionary! {
            "Title" => Object::string_literal("Subsection"), "A" => dictionary! { "S" => "GoTo", "D" => destination(first) },
        }));
        let outline = doc.add_object(dictionary! { "First" => top });
        let catalog = doc.get_object_mut(root).unwrap().as_dict_mut().unwrap();
        catalog.set("Names", dictionary! { "Dests" => names });
        catalog.set("Outlines", outline);
        doc.get_object_mut(first).unwrap().as_dict_mut().unwrap().set("Annots", vec![
            link(destination(second)), link(Object::string_literal("chapter")),
            Object::Dictionary(dictionary! { "Type" => "Annot", "Subtype" => "Link",
                "Rect" => vec![10.into(), 50.into(), 100.into(), 70.into()],
                "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.com") } }),
        ]);
        let nav = navigation_for_document(&doc, 2).unwrap();
        assert_eq!(nav.links[0].len(), 3);
        assert_eq!(nav.links[0][0].destination_page, Some(1));
        assert_eq!(nav.links[0][1].destination_page, Some(1));
        assert_eq!(nav.links[0][2].url, "https://example.com");
        assert_eq!(
            nav.outline
                .iter()
                .map(|item| (&*item.title, item.page_index, item.level))
                .collect::<Vec<_>>(),
            vec![("Chapter", Some(1), 0), ("Subsection", Some(0), 1)]
        );
    }

    #[test]
    fn cyclic_and_invalid_destinations_are_ignored() {
        let (mut doc, _root, first, _second) = document();
        let cycle = doc.new_object_id();
        doc.objects.insert(cycle, Object::Reference(cycle));
        doc.get_object_mut(first).unwrap().as_dict_mut().unwrap().set("Annots", vec![
            link(Object::Reference(cycle)), link(Object::Integer(99)),
            Object::Dictionary(dictionary! { "Subtype" => "Link", "Rect" => vec![1.into(), 1.into(), 3.into(), 3.into()],
                "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("javascript:alert(1)") } }),
        ]);
        let nav = navigation_for_document(&doc, 2).unwrap();
        assert!(nav.links[0].is_empty());
    }

    #[test]
    fn legacy_names_and_name_tree_children_are_resolved_without_external_actions() {
        let (mut doc, root, first, second) = document();
        let leaf = doc.add_object(dictionary! { "Names" => vec![Object::string_literal("from-tree"), destination(second)] });
        let tree = doc.new_object_id();
        doc.objects.insert(
            tree,
            Object::Dictionary(
                dictionary! { "Kids" => vec![Object::Reference(leaf), Object::Reference(tree)] },
            ),
        );
        let catalog = doc.get_object_mut(root).unwrap().as_dict_mut().unwrap();
        catalog.set("Dests", dictionary! { "legacy" => destination(first) });
        catalog.set("Names", dictionary! { "Dests" => tree });
        doc.get_object_mut(first).unwrap().as_dict_mut().unwrap().set("Annots", vec![
            link(Object::Name(b"legacy".to_vec())),
            link(Object::string_literal("from-tree")),
            Object::Dictionary(dictionary! { "Subtype" => "Link", "Rect" => vec![1.into(), 1.into(), 3.into(), 3.into()],
                "A" => dictionary! { "S" => "GoToR", "F" => Object::string_literal("other.pdf"), "D" => destination(second) } }),
        ]);
        let nav = navigation_for_document(&doc, 2).unwrap();
        assert_eq!(
            nav.links[0]
                .iter()
                .map(|link| link.destination_page)
                .collect::<Vec<_>>(),
            vec![Some(0), Some(1)]
        );
    }

    #[test]
    fn resolves_positions_on_cropped_and_rotated_pages() {
        for (rotation, xyz, fit_rect) in [
            (0, 0.25, 0.25),
            (90, 1.0 / 6.0, 1.0 / 6.0),
            (180, 0.75, 0.25),
            (270, 5.0 / 6.0, 0.5),
        ] {
            let (mut doc, _root, first, second) = document();
            let target = doc.get_object_mut(second).unwrap().as_dict_mut().unwrap();
            target.set("MediaBox", vec![0.into(), 0.into(), 700.into(), 900.into()]);
            target.set(
                "CropBox",
                vec![20.into(), 40.into(), 620.into(), 840.into()],
            );
            target.set("Rotate", rotation);
            let make = |kind: &[u8], args: Vec<Object>| {
                let mut items = vec![Object::Reference(second), Object::Name(kind.to_vec())];
                items.extend(args);
                link(Object::Array(items))
            };
            let half_fit = if rotation == 0 || rotation == 180 {
                make(b"FitH", vec![640.into()])
            } else {
                make(b"FitV", vec![120.into()])
            };
            let missing = if rotation == 0 || rotation == 180 {
                make(b"XYZ", vec![120.into(), Object::Null, Object::Null])
            } else {
                make(b"XYZ", vec![Object::Null, 640.into(), Object::Null])
            };
            doc.get_object_mut(first)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set(
                    "Annots",
                    vec![
                        make(b"XYZ", vec![120.into(), 640.into(), Object::Null]),
                        half_fit,
                        make(
                            b"FitR",
                            vec![120.into(), 240.into(), 320.into(), 640.into()],
                        ),
                        missing,
                        make(b"Fit", vec![]),
                    ],
                );
            let nav = navigation_for_document(&doc, 2).unwrap();
            let fractions: Vec<_> = nav.links[0]
                .iter()
                .map(|link| link.destination_fraction)
                .collect();
            assert_eq!(fractions.len(), 5);
            for (actual, expected) in fractions[..3].iter().zip([xyz, xyz, fit_rect]) {
                assert!(
                    (actual.unwrap() - expected).abs() < 0.00001,
                    "rotation {rotation}: {actual:?} vs {expected}"
                );
            }
            assert_eq!(fractions[3..], [None, None]);
        }
    }

    #[test]
    fn named_destination_preserves_position_for_links_and_outline() {
        let (mut doc, root, first, second) = document();
        let named = Object::Array(vec![
            Object::Reference(second),
            Object::Name(b"FitH".to_vec()),
            600.into(),
        ]);
        let names = doc
            .add_object(dictionary! { "Names" => vec![Object::string_literal("position"), named] });
        let item = doc.add_object(dictionary! { "Title" => Object::string_literal("Position"), "Dest" => Object::Name(b"position".to_vec()) });
        let outline = doc.add_object(dictionary! { "First" => item });
        let catalog = doc.get_object_mut(root).unwrap().as_dict_mut().unwrap();
        catalog.set("Names", dictionary! { "Dests" => names });
        catalog.set("Outlines", outline);
        doc.get_object_mut(first)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", vec![link(Object::string_literal("position"))]);
        let nav = navigation_for_document(&doc, 2).unwrap();
        let expected = 1.0 - 600.0 / 792.0;
        assert!((nav.links[0][0].destination_fraction.unwrap() - expected).abs() < 0.00001);
        assert!((nav.outline[0].page_fraction.unwrap() - expected).abs() < 0.00001);
    }
}

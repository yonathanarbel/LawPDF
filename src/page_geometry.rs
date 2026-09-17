//! Coordinates exposed to the editor are the visible, rotated page, with a
//! bottom-left origin. PDF annotation objects retain the original PDF space.
use crate::model::{AnnotationKind, EditorAnnotation, PdfRect};

#[derive(Clone, Copy, Debug)]
pub struct PageGeometry {
    pub bounds: PdfRect,
    pub rotation: i64,
}

impl PageGeometry {
    pub fn to_display(self, point: (f32, f32)) -> (f32, f32) {
        let (x, y) = (point.0 - self.bounds.left, point.1 - self.bounds.bottom);
        let (w, h) = (self.bounds.width(), self.bounds.height());
        match self.rotation.rem_euclid(360) {
            90 => (y, w - x),
            180 => (w - x, h - y),
            270 => (h - y, x),
            _ => (x, y),
        }
    }

    pub fn to_pdf(self, point: (f32, f32)) -> (f32, f32) {
        let (x, y) = point;
        let (w, h) = (self.bounds.width(), self.bounds.height());
        let (x, y) = match self.rotation.rem_euclid(360) {
            90 => (w - y, x),
            180 => (w - x, h - y),
            270 => (y, h - x),
            _ => (x, y),
        };
        (x + self.bounds.left, y + self.bounds.bottom)
    }

    pub fn rect(self, rect: PdfRect, to_pdf: bool) -> PdfRect {
        let map = |point| {
            if to_pdf {
                self.to_pdf(point)
            } else {
                self.to_display(point)
            }
        };
        let a = map((rect.left, rect.bottom));
        let b = map((rect.right, rect.top));
        PdfRect::new(a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
    }

    pub fn annotation(self, annotation: &mut EditorAnnotation, to_pdf: bool) {
        annotation.rect = self.rect(annotation.rect, to_pdf);
        let map = |point| {
            if to_pdf {
                self.to_pdf(point)
            } else {
                self.to_display(point)
            }
        };
        match &mut annotation.kind {
            AnnotationKind::Comment { anchor, .. } => *anchor = map(*anchor),
            AnnotationKind::Signature { strokes, .. } => {
                for point in strokes.iter_mut().flatten() {
                    *point = map(*point);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cropped_rotated_pages_round_trip_points_and_rectangles() {
        for rotation in [0, 90, 180, 270] {
            let geometry = PageGeometry {
                bounds: PdfRect::new(20.0, 40.0, 620.0, 840.0),
                rotation,
            };
            for point in [(20.0, 40.0), (100.0, 200.0), (620.0, 840.0)] {
                assert_eq!(geometry.to_pdf(geometry.to_display(point)), point);
            }
            let rect = PdfRect::new(40.0, 90.0, 150.0, 300.0);
            assert_eq!(geometry.rect(geometry.rect(rect, false), true), rect);
        }
    }
    #[test]
    fn clockwise_rotation_moves_the_bottom_left_to_top_left() {
        let geometry = PageGeometry {
            bounds: PdfRect::new(0.0, 0.0, 600.0, 800.0),
            rotation: 90,
        };
        assert_eq!(geometry.to_display((0.0, 0.0)), (0.0, 600.0));
        assert_eq!(geometry.to_display((600.0, 800.0)), (800.0, 0.0));
    }
}

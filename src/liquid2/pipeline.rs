//! The Review Mode pipeline driver: extraction to decoded lines to assembled document, plus article-span glue.

use super::*;

pub fn prepare_liquid_mode2_document(
    request: LiquidMode2Request,
) -> Result<LiquidDocument, String> {
    prepare_liquid_mode2_document_with_timing(request).map(|(document, _)| document)
}

pub fn prepare_liquid_mode2_document_with_timing(
    request: LiquidMode2Request,
) -> Result<(LiquidDocument, LiquidMode2Timing), String> {
    let total_started = Instant::now();
    let runtime_started = Instant::now();
    let mut runtime_lease = Lm2RuntimeLease::acquire(request.runtime_choice);
    let runtime = runtime_lease.runtime();
    let mut pp_runtime_warnings = Vec::new();
    let mut liquidvision_runtime_warnings = Vec::new();
    if request.use_pp_footnote_regions {
        runtime.pp_footnote_region_membership = true;
        if runtime.pp_priors.is_none() {
            match load_or_generate_lm2_pp_priors(&request.path, &request.deep_source_lines) {
                Ok(Some(index)) => {
                    runtime.pp_priors = Some(index);
                }
                Ok(None) => {
                    runtime.pp_footnote_region_membership = false;
                    pp_runtime_warnings
                        .push("PP-DocLayout runtime produced no usable LM2 prior rows.".to_owned());
                }
                Err(error) => {
                    runtime.pp_footnote_region_membership = false;
                    pp_runtime_warnings.push(format!(
                        "PP-DocLayout runtime unavailable; using base LM2 model: {error}"
                    ));
                }
            }
        }
        if runtime.pp_footnote_region_membership
            && !runtime
                .pp_priors
                .as_ref()
                .is_some_and(lm2_pp_prior_index_has_footnotes)
        {
            runtime.pp_footnote_region_membership = false;
            if runtime.pp_priors.is_some() {
                pp_runtime_warnings.push(
                    "PP-DocLayout runtime produced no high-confidence footnote rows for LM2."
                        .to_owned(),
                );
            }
        }
    }
    let mut timing = LiquidMode2Timing {
        runtime_load_ms: runtime_started.elapsed().as_secs_f64() * 1000.0,
        ..Default::default()
    };
    let native_line_model_no_stack = runtime.native_line_model_active();
    let d1_runtime_zerospend_overlay =
        !native_line_model_no_stack && lm2_d1_runtime_zerospend_overlay_enabled();
    let d1_runtime_continuation_overlay =
        !native_line_model_no_stack && lm2_d1_runtime_continuation_overlay_enabled();
    let d1_runtime_immediate_continuation_overlay =
        !native_line_model_no_stack && lm2_d1_runtime_immediate_continuation_overlay_enabled();
    let d1_runtime_sandwiched_continuation_overlay =
        !native_line_model_no_stack && lm2_d1_runtime_sandwiched_continuation_overlay_enabled();
    let d1_runtime_wide_sandwich_overlay =
        !native_line_model_no_stack && lm2_d1_runtime_wide_sandwich_overlay_enabled();
    let d1_runtime_safe_numeric_note_overlay =
        !native_line_model_no_stack && lm2_d1_runtime_safe_numeric_note_overlay_enabled();
    let d1_runtime_post_wide_cue_overlay =
        !native_line_model_no_stack && lm2_d1_runtime_post_wide_cue_overlay_enabled();
    let d1_runtime_postcue_citation_next1_overlay =
        !native_line_model_no_stack && lm2_d1_runtime_postcue_citation_next1_overlay_enabled();
    let d1_runtime_near8_cue_overlay =
        !native_line_model_no_stack && lm2_d1_runtime_near8_cue_overlay_enabled();
    let d1_runtime_wide_divider_guard_overlay =
        !native_line_model_no_stack && lm2_d1_runtime_wide_divider_guard_overlay_enabled();
    let d1_runtime_geometric_zone_overlay =
        !native_line_model_no_stack && lm2_d1_runtime_geometric_zone_overlay_enabled();
    let d1_runtime_footer_artifact_overlay =
        !native_line_model_no_stack && lm2_d1_runtime_footer_artifact_overlay_enabled();
    let footnote_monotone_overlay =
        !native_line_model_no_stack && lm2_footnote_monotone_overlay_enabled();
    let footnote_carryover_overlay =
        !native_line_model_no_stack && lm2_footnote_carryover_overlay_enabled();
    let open_footnote_carryover_overlay = lm2_open_footnote_carryover_overlay_enabled();
    let table_figure_router_overlay =
        !native_line_model_no_stack && lm2_table_figure_router_overlay_enabled();
    let page_object_overlay = !native_line_model_no_stack && lm2_page_object_overlay_enabled();
    let page_object_tuned_overlay =
        !native_line_model_no_stack && lm2_page_object_tuned_overlay_enabled();
    let d1_runtime_zerospend_overlay_version =
        d1_runtime_zerospend_overlay.then_some(LM2_D1_RUNTIME_ZEROSPEND_OVERLAY_VERSION);
    let context_twopass_label = if request.external_emissions_path.is_none() {
        let mut labels = [
            runtime.context_twopass_model.as_ref(),
            runtime.context_arbiter_model.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(Lm2ContextTwopassModel::label)
        .collect::<Vec<_>>();
        if let Some(model) = runtime.note_head_model.as_ref() {
            labels.push(model.label());
        }
        if let Some(model) = runtime.link_ranker_model.as_ref() {
            labels.push(model.label());
        }
        (!labels.is_empty()).then(|| labels.join(" -> "))
    } else {
        None
    };
    let source_signature = lm2_source_signature(
        &request.path,
        &request.pages,
        &runtime.model_label,
        context_twopass_label.as_deref(),
        runtime.pp_prior_source().as_deref(),
        runtime
            .static_front_overlay
            .as_ref()
            .map(|overlay| overlay.source_label.as_str()),
        request.use_pymupdf_blocks,
        runtime.pp_footnote_region_membership,
        runtime.marker_decoder_prior,
        runtime.small_font_decoder_prior,
        runtime.small_font_sequence_prior,
        runtime.anchored_marginalia_flow_guard,
        runtime.body_preservation_guard,
        runtime.action_neutral_blocksplit,
        runtime.toc_overlay,
        runtime.front_matter_guard,
        runtime.marginalia_preservation_guard,
        d1_runtime_zerospend_overlay,
        d1_runtime_zerospend_overlay_version,
        d1_runtime_continuation_overlay,
        d1_runtime_immediate_continuation_overlay,
        d1_runtime_sandwiched_continuation_overlay,
        d1_runtime_wide_sandwich_overlay,
        d1_runtime_safe_numeric_note_overlay,
        d1_runtime_post_wide_cue_overlay,
        d1_runtime_postcue_citation_next1_overlay,
        d1_runtime_near8_cue_overlay,
        d1_runtime_wide_divider_guard_overlay,
        d1_runtime_geometric_zone_overlay,
        d1_runtime_footer_artifact_overlay,
        footnote_monotone_overlay,
        footnote_carryover_overlay,
        table_figure_router_overlay,
        page_object_overlay,
        page_object_tuned_overlay,
        runtime.start_score_scale,
        runtime.transition_score_scale,
    );
    let external_emissions = request
        .external_emissions_path
        .as_deref()
        .map(Lm2ExternalEmissions::load)
        .transpose()?;
    if external_emissions.is_none()
        && !open_footnote_carryover_overlay
        && let Some(cached) = load_cached_lm2_document(&source_signature)
    {
        if !cached.blocks.is_empty() && !cached.block_source_lines.is_empty() {
            timing.cache_hit = true;
            timing.total_ms = total_started.elapsed().as_secs_f64() * 1000.0;
            return Ok((cached, timing));
        }
    }

    let feature_started = Instant::now();
    let mut lines = request
        .deep_source_lines
        .iter()
        .filter(|line| !line.text.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    if native_line_model_no_stack
        && liquidvision_enabled(true)
        && !lines.iter().all(|line| line.lv.has_region)
    {
        match PdfEngine::new()
            .map_err(|error| error.to_string())
            .and_then(|engine| {
                fill_document_features(&engine, &request.path, request.pages.len(), &mut lines)
                    .map_err(|error| error.to_string())
            }) {
            Ok(report) => {
                timing.liquidvision_fill_ms = report.elapsed_ms;
                if report.pages_filled != report.pages_attempted || !report.errors.is_empty() {
                    liquidvision_runtime_warnings.push(format!(
                        "LiquidVision populated {}/{} pages; {} page error(s): {}",
                        report.pages_filled,
                        report.pages_attempted,
                        report.errors.len(),
                        report.errors.join("; ")
                    ));
                }
            }
            Err(error) => liquidvision_runtime_warnings.push(format!(
                "LiquidVision runtime feature fill failed; native model received zero vision features: {error}"
            )),
        }
    } else if native_line_model_no_stack && !liquidvision_enabled(true) {
        liquidvision_runtime_warnings.push(
            "LiquidVision was disabled; native model received zero vision features.".to_owned(),
        );
    }
    annotate_pp_priors_for_lines(&runtime, &request.path.display().to_string(), &mut lines);
    enrich_lm2_document_features(&mut lines);
    timing.feature_enrichment_ms =
        (feature_started.elapsed().as_secs_f64() * 1000.0 - timing.liquidvision_fill_ms).max(0.0);
    if lines.is_empty() {
        let document = LiquidDocument {
            title: request.title,
            blocks: Vec::new(),
            article_spans: Vec::new(),
            block_source_lines: Vec::new(),
            footnote_links: Vec::new(),
            footnote_link_integrity: None,
            profile: Some(lm2_profile()),
            noise_lines_removed: 0,
            llm_used: false,
            llm_provider: Some("LM2".to_owned()),
            deep_liquid_used: false,
            deep_liquid_model: Some(runtime.model_label.clone()),
            warnings: vec![
                "LiquidMode2 found no selectable text. Run OCR before using LM2.".to_owned(),
            ],
            source_signature,
        };
        if external_emissions.is_none() && !open_footnote_carryover_overlay {
            let _ = save_cached_lm2_document(&document);
        }
        timing.total_ms = total_started.elapsed().as_secs_f64() * 1000.0;
        return Ok((document, timing));
    }

    let model_started = Instant::now();
    let (mut decoded, primary_emissions) = if let Some(external) = external_emissions.as_ref() {
        (
            decode_pages_with_external_emissions(&runtime, &request.path, &lines, external)?,
            None,
        )
    } else {
        let (decoded, emissions) = decode_pages_with_scores(&runtime, &lines)?;
        (decoded, Some(emissions))
    };
    timing.model_decode_ms = model_started.elapsed().as_secs_f64() * 1000.0;

    let overlay_started = Instant::now();
    if external_emissions.is_none()
        && let Some(model) = runtime.context_twopass_model.as_ref()
    {
        apply_context_twopass_model(
            model,
            &request.path,
            primary_emissions.as_deref(),
            &mut decoded,
        );
    }
    if external_emissions.is_none()
        && let Some(model) = runtime.context_arbiter_model.as_ref()
    {
        apply_context_twopass_model(
            model,
            &request.path,
            primary_emissions.as_deref(),
            &mut decoded,
        );
    }
    if let Some(overlay) = runtime.static_front_overlay.as_ref() {
        apply_static_front_overlay(overlay, &request.path, &mut decoded);
    }
    if runtime.toc_overlay {
        apply_document_toc_overlay(&mut decoded);
    }
    if runtime.front_matter_guard {
        apply_front_matter_guard(&mut decoded);
    }
    if runtime.marginalia_preservation_guard {
        apply_marginalia_preservation_guard(&mut decoded);
    }
    if d1_runtime_zerospend_overlay {
        apply_d1_runtime_zerospend_overlay(&mut decoded);
    }
    if d1_runtime_continuation_overlay {
        apply_d1_runtime_continuation_overlay(&mut decoded);
    }
    if d1_runtime_immediate_continuation_overlay {
        apply_d1_runtime_immediate_continuation_overlay(&mut decoded);
    }
    if d1_runtime_sandwiched_continuation_overlay {
        apply_d1_runtime_sandwiched_continuation_overlay(&mut decoded);
    }
    if d1_runtime_wide_sandwich_overlay {
        apply_d1_runtime_wide_sandwich_overlay(&mut decoded);
    }
    if d1_runtime_safe_numeric_note_overlay {
        apply_d1_runtime_safe_numeric_note_overlay(&mut decoded);
    }
    if d1_runtime_post_wide_cue_overlay {
        apply_d1_runtime_post_wide_cue_overlay(&mut decoded);
    }
    if d1_runtime_postcue_citation_next1_overlay {
        apply_d1_runtime_postcue_citation_next1_overlay(&mut decoded);
    }
    if d1_runtime_near8_cue_overlay {
        apply_d1_runtime_near8_cue_overlay(&mut decoded);
    }
    if d1_runtime_wide_divider_guard_overlay {
        apply_d1_runtime_wide_divider_guard_overlay(&mut decoded);
    }
    if table_figure_router_overlay {
        apply_table_figure_router_overlay(&mut decoded);
    }
    if page_object_overlay {
        apply_page_object_overlay(&mut decoded);
    }
    if page_object_tuned_overlay {
        apply_page_object_tuned_overlay(&mut decoded);
    }
    if d1_runtime_geometric_zone_overlay {
        apply_d1_runtime_geometric_zone_overlay(&mut decoded);
    }
    if d1_runtime_footer_artifact_overlay {
        apply_d1_runtime_footer_artifact_overlay(&mut decoded);
    }
    if footnote_carryover_overlay {
        apply_footnote_carryover_overlay(&mut decoded);
    }
    if footnote_monotone_overlay {
        // Applied after article spans are known so bound volumes do not share
        // one file-wide missing-marker chain.
    }
    if open_footnote_carryover_overlay {
        apply_open_footnote_carryover_overlay(&mut decoded);
    }
    if !native_line_model_no_stack {
        apply_page_label_furniture_guard(&mut decoded);
    }
    apply_synthetic_ocr_body_preservation(&mut decoded);
    apply_front_matter_abstract_recovery(&mut decoded);
    apply_drop_cap_recovery_overlay(&mut decoded);
    apply_same_row_body_fragment_overlay(&mut decoded);
    apply_same_page_body_callout_overlay(&mut decoded);
    if let (Some(link_model), Some(auth_model)) = (
        runtime.link_ranker_model.as_ref(),
        runtime.note_head_model.as_ref(),
    ) {
        let linker_started = Instant::now();
        apply_footnote_link_ranker(
            link_model,
            auth_model,
            &request.path,
            primary_emissions.as_deref(),
            &mut decoded,
        );
        timing.footnote_linker_ms = linker_started.elapsed().as_secs_f64() * 1000.0;
    } else if let Some(model) = runtime.note_head_model.as_ref() {
        apply_note_head_model(
            model,
            &request.path,
            primary_emissions.as_deref(),
            &mut decoded,
        );
    }
    apply_hidden_numbered_note_head_recovery(&mut decoded);
    apply_explicit_endnote_section_guard(&mut decoded);
    apply_preceding_small_font_note_continuation_guard(&mut decoded);
    // The note-head model resolves definitions that were still ambiguous when
    // the early body overlay ran. Re-run the guarded page sequence now so
    // flattened multi-digit callouts can use those newly established heads
    // before block assembly's more permissive one-digit recovery.
    apply_scanned_glyph_note_sequence_bridge(&request.path, &mut decoded);
    apply_decoded_page_sequence_callout_recovery(&mut decoded);
    apply_repository_cover_guard(&mut decoded);
    apply_strict_no_dot_toc_page_guard(&mut decoded);
    apply_dense_dotleader_toc_run_guard(&mut decoded);
    apply_final_assembly_safety_guards(&mut decoded);
    apply_isolated_body_noise_tail_recovery(&mut decoded);
    apply_numbered_table_figure_band_role_hints(&mut decoded);
    // The final furniture guard can still inherit a stale Noise hint for a
    // terse `N. Id.` line. Reassert the narrower physical note-head rule after
    // every line-level overlay has finished.
    apply_hidden_numbered_note_head_recovery(&mut decoded);
    apply_same_segment_statutory_subdivision_body_rescue(&mut decoded);
    let article_spans = detect_lm2_article_spans(&decoded, request.pages.len());
    if footnote_monotone_overlay {
        apply_footnote_monotone_overlay(&mut decoded, &article_spans);
    }
    apply_document_discretionary_hyphen_repairs(&mut decoded);
    apply_document_fused_word_spacing_repairs(&mut decoded);
    timing.overlay_decode_ms = overlay_started.elapsed().as_secs_f64() * 1000.0;

    let assembly_started = Instant::now();
    let hidden_count = decoded
        .iter()
        .filter(|(_, action)| *action == Lm2Action::HideNoise)
        .count();
    let grouping = if request.use_pymupdf_blocks {
        try_apply_lm2_pymupdf_grouping(
            &request.path,
            &request.title,
            &source_signature,
            &request.deep_source_lines,
        )
        .ok()
        .flatten()
    } else {
        None
    };
    let (title, mut blocks, mut block_source_lines) =
        build_lm2_blocks_with_grouping(&request.title, &decoded, grouping.as_ref(), &article_spans);
    apply_leading_letterspaced_author_recovery(&mut blocks, &mut block_source_lines);
    apply_false_marginalia_note_head_reflow(&mut blocks, &mut block_source_lines);
    if runtime.action_neutral_blocksplit {
        apply_action_neutral_blocksplit(&mut blocks, &mut block_source_lines, &decoded);
    }
    apply_heading_outline_splits(&mut blocks, &mut block_source_lines, &decoded);
    apply_heading_continuation_reflow(&mut blocks, &mut block_source_lines, &decoded);
    apply_heading_leading_source_continuation_split(&mut blocks, &mut block_source_lines, &decoded);
    apply_front_matter_abstract_author_reflow(&mut blocks, &mut block_source_lines);
    apply_front_matter_byline_abstract_split(&mut blocks, &mut block_source_lines);
    apply_hidden_numbered_note_block_recovery(&mut blocks, &mut block_source_lines, &decoded);
    apply_hidden_numbered_note_block_split_recovery(&mut blocks, &mut block_source_lines, &decoded);
    // Hidden-note recovery can expose the true head immediately before a
    // citation pincite that was itself mistaken for a note number.  Re-run
    // the provenance-backed reflow now that the true head is visible.
    apply_false_marginalia_note_head_reflow(&mut blocks, &mut block_source_lines);
    apply_table_figure_block_role_routing(&mut blocks, &block_source_lines, &decoded);
    // The first pass establishes the main table cluster. A repeated header on
    // the next page can only be recognized after that cluster is visible.
    apply_table_figure_block_role_routing(&mut blocks, &block_source_lines, &decoded);
    apply_front_matter_contents_block_suppression(&mut blocks, &mut block_source_lines);
    apply_interleaved_note_continuation_reflow(&mut blocks, &mut block_source_lines, &decoded);
    apply_inline_body_callout_marginalia_reflow(&mut blocks, &mut block_source_lines, &decoded);
    apply_geometry_backed_leading_callout_bridge(&mut blocks, &mut block_source_lines, &decoded);
    apply_mixed_marginalia_leading_body_callout_split(
        &mut blocks,
        &mut block_source_lines,
        &decoded,
    );
    apply_contiguous_body_marginalia_reflow(&mut blocks, &mut block_source_lines, &decoded);
    apply_deferred_marginalia_reflow(&mut blocks, &mut block_source_lines);
    apply_above_note_body_marginalia_rescue(&mut blocks, &block_source_lines, &decoded);
    apply_body_callout_marginalia_rescue(&mut blocks, &block_source_lines, &decoded);
    apply_physical_sequential_note_head_recovery(&mut blocks, &mut block_source_lines, &decoded);
    apply_recovered_trailing_note_marker_reflow(&mut blocks, &mut block_source_lines, &decoded);
    apply_local_monotone_ascii_callout_recovery(&mut blocks, &mut block_source_lines, &decoded);
    apply_mixed_marginalia_leading_body_callout_split(
        &mut blocks,
        &mut block_source_lines,
        &decoded,
    );
    apply_page_sequence_ascii_callout_recovery(&mut blocks, &mut block_source_lines);
    // The page matcher establishes additional accepted heads. Feed that
    // stronger provenance back through the exact-successor fixed point, then
    // recover locally bracketed whitespace-separated body callouts.
    apply_body_backed_bare_note_head_recovery(&mut blocks, &mut block_source_lines, &decoded);
    apply_physical_sequential_note_head_recovery(&mut blocks, &mut block_source_lines, &decoded);
    apply_recovered_trailing_note_marker_reflow(&mut blocks, &mut block_source_lines, &decoded);
    apply_local_monotone_ascii_callout_recovery(&mut blocks, &mut block_source_lines, &decoded);
    apply_mixed_marginalia_leading_body_callout_split(
        &mut blocks,
        &mut block_source_lines,
        &decoded,
    );
    apply_attached_terminal_callout_recovery(&mut blocks, &block_source_lines);
    apply_in_block_standalone_callout_recovery(&mut blocks, &block_source_lines, &decoded);
    apply_same_row_leading_callout_reflow(&mut blocks, &mut block_source_lines, &decoded);
    apply_numeric_footer_furniture_suppression(&mut blocks, &block_source_lines, &decoded);
    // Rescue passes can demote a source-backed outline label while fixing its
    // surrounding body flow. Reapply the provenance-gated splitter only after
    // all such role mutations are complete.
    apply_heading_outline_splits(&mut blocks, &mut block_source_lines, &decoded);
    apply_sandwiched_numbered_outline_recovery(&mut blocks, &mut block_source_lines);
    // Later body/marginalia rescue passes can conservatively demote a true
    // single-line definition that the early hidden-note pass recovered.  Make
    // provenance-backed note heads and adjacent false pincites stable at the
    // final block boundary before linking.
    apply_hidden_numbered_note_block_recovery(&mut blocks, &mut block_source_lines, &decoded);
    apply_false_marginalia_note_head_reflow(&mut blocks, &mut block_source_lines);
    // Later rescue passes can be what finally settles the preceding physical
    // note as Marginalia. Re-run the provenance-backed continuation repair at
    // that stable boundary so an interleaved small-font line cannot remain in
    // body prose merely because its note target was still ambiguous earlier.
    apply_sandwiched_body_noise_reflow(&mut blocks, &mut block_source_lines, &decoded);
    apply_source_gap_body_line_reflow(&mut blocks, &mut block_source_lines, &decoded);
    apply_embedded_small_font_note_continuation_split(
        &mut blocks,
        &mut block_source_lines,
        &decoded,
    );
    // Late source-gap and small-font repairs can settle a body-shaped quote as
    // markerless Marginalia only after the first above-note rescue ran. Give
    // that same provenance-gated rescue one final pass before note reflow.
    apply_above_note_body_marginalia_rescue(&mut blocks, &block_source_lines, &decoded);
    apply_interleaved_note_continuation_reflow(&mut blocks, &mut block_source_lines, &decoded);
    apply_final_source_backed_paragraph_splits(&mut blocks, &mut block_source_lines, &decoded);
    // A newly exposed paragraph boundary can also reveal that the following
    // source line is a numbered note head. Re-run provenance-backed role
    // recovery so the split cannot strand that definition in body prose.
    apply_hidden_numbered_note_block_recovery(&mut blocks, &mut block_source_lines, &decoded);
    apply_false_marginalia_note_head_reflow(&mut blocks, &mut block_source_lines);
    // A same-row inline callout can only use its accepted definition after the
    // late hidden-note pass has recovered that definition's provenance. Re-run
    // the idempotent bridge here so the adjacent lowercase body row is joined
    // before the final source-backed splitter.
    apply_same_row_leading_callout_reflow(&mut blocks, &mut block_source_lines, &decoded);
    apply_final_physical_footnote_segment_guard(&mut blocks, &mut block_source_lines, &decoded);
    // The final source-backed split can likewise expose the middle of a
    // cross-page footnote segment as a standalone Paragraph. Run the guarded
    // sequential-boundary closure to a tiny fixed point at this stable block
    // boundary: the first pass can settle the adjacent markerless segment or
    // numbered target that the second pass then uses as its boundary proof.
    for _ in 0..2 {
        if apply_interleaved_note_continuation_reflow(
            &mut blocks,
            &mut block_source_lines,
            &decoded,
        ) == 0
        {
            break;
        }
    }
    // Some body-shaped displays and top-of-page note continuations only have
    // stable ownership after the final note fixed point.  Re-assert those
    // source-backed roles here, after no broad rescue pass remains.
    apply_final_displayed_body_role_rescue(&mut blocks, &mut block_source_lines, &decoded);
    apply_final_mixed_furniture_prefix_suppression(&mut blocks, &mut block_source_lines, &decoded);
    apply_final_physical_footnote_segment_guard(&mut blocks, &mut block_source_lines, &decoded);
    for _ in 0..2 {
        if apply_interleaved_note_continuation_reflow(
            &mut blocks,
            &mut block_source_lines,
            &decoded,
        ) == 0
        {
            break;
        }
    }
    // Nothing note-oriented runs after this point, so a rescued display cannot
    // be pulled back into a numbered definition.
    apply_final_displayed_body_role_rescue(&mut blocks, &mut block_source_lines, &decoded);
    // The native model can emit one continuation row immediately before its
    // numbered head in block order even though the source coordinates are
    // head, continuation, then the remainder of the note. Run this only after
    // the final role/split fixed point, where those three blocks are stable.
    apply_inverted_numbered_note_continuation_reflow(
        &mut blocks,
        &mut block_source_lines,
        &decoded,
    );
    apply_final_caption_continuation_reflow(&mut blocks, &mut block_source_lines, &decoded);
    apply_final_noise_inline_callout_bridge(&mut blocks, &mut block_source_lines, &decoded);
    apply_final_fused_heading_body_splits(&mut blocks, &mut block_source_lines, &decoded);
    apply_final_body_source_order_and_coalescing(&mut blocks, &mut block_source_lines, &decoded);
    // Reassert source-backed display boundaries after the first ordering pass,
    // then run the same guarded coalescer once more. The splitter can expose a
    // wrapped body row that was not a separate block during the first pass;
    // the coalescer already treats displayed labels, font cliffs, and genuine
    // paragraph boundaries as hard stops.
    apply_final_source_backed_paragraph_splits(&mut blocks, &mut block_source_lines, &decoded);
    apply_final_body_source_order_and_coalescing(&mut blocks, &mut block_source_lines, &decoded);
    // Keep displayed labels separated after the last flow join. The splitter
    // explicitly ignores source-proven lowercase wraps, so this pass is
    // idempotent for repaired body flow while retaining real definitions.
    apply_final_source_backed_paragraph_splits(&mut blocks, &mut block_source_lines, &decoded);
    // The last splitter can expose a source-role boundary (for example a
    // callout-bearing row tagged Heading or Marginalia) even though the exact
    // adjacent source geometry proves one lowercase body flow. Nothing after
    // this pass can split the repaired flow again.
    apply_final_body_source_order_and_coalescing(&mut blocks, &mut block_source_lines, &decoded);
    // Reassert the strongest paired note evidence only after every broad
    // role/reflow pass has finished. A local callout rewrite may expose one
    // last body join, so coalesce once and then reassert definition markers at
    // the terminal boundary where nothing can demote them again.
    apply_body_backed_bare_note_head_recovery(&mut blocks, &mut block_source_lines, &decoded);
    apply_physical_sequential_note_head_recovery(&mut blocks, &mut block_source_lines, &decoded);
    apply_recovered_trailing_note_marker_reflow(&mut blocks, &mut block_source_lines, &decoded);
    apply_local_monotone_ascii_callout_recovery(&mut blocks, &mut block_source_lines, &decoded);
    apply_mixed_marginalia_leading_body_callout_split(
        &mut blocks,
        &mut block_source_lines,
        &decoded,
    );
    apply_final_body_source_order_and_coalescing(&mut blocks, &mut block_source_lines, &decoded);
    apply_body_backed_bare_note_head_recovery(&mut blocks, &mut block_source_lines, &decoded);
    apply_physical_sequential_note_head_recovery(&mut blocks, &mut block_source_lines, &decoded);
    apply_recovered_trailing_note_marker_reflow(&mut blocks, &mut block_source_lines, &decoded);
    apply_attached_terminal_callout_recovery(&mut blocks, &block_source_lines);
    apply_mixed_marginalia_leading_body_callout_split(
        &mut blocks,
        &mut block_source_lines,
        &decoded,
    );
    apply_terminal_out_of_order_callout_fragment_reflow(&mut blocks, &block_source_lines, &decoded);
    apply_source_backed_dotted_numeric_callout_restoration(&mut blocks, &block_source_lines);
    apply_leading_callout_backfill(&mut blocks, &block_source_lines);
    apply_adjacent_duplicate_callout_suppression(&mut blocks);
    apply_sandwiched_note_url_continuation_reflow(&mut blocks, &mut block_source_lines, &decoded);
    apply_final_source_backed_figure_caption_isolation(
        &mut blocks,
        &mut block_source_lines,
        &decoded,
    );
    apply_final_body_source_order_and_coalescing(&mut blocks, &mut block_source_lines, &decoded);
    // The terminal body coalescer can finally place a low-font statutory
    // subdivision directly between its colon lead-in and numbered children.
    // Re-run the structural display rescue at this stable boundary so the
    // Markdown note assembler cannot absorb that body display into an open
    // cross-page footnote.
    apply_final_displayed_body_role_rescue(&mut blocks, &mut block_source_lines, &decoded);
    rescue_omitted_keep_source_lines(&mut blocks, &mut block_source_lines, &decoded);
    let mut warnings = if runtime.native_line_model_active() {
        vec![format!(
            "Review Mode uses {} raw no-stack emissions.",
            runtime.model_label
        )]
    } else {
        vec![format!(
            "Review Mode uses {} emissions plus page-level sequence decoder.",
            runtime.model_label
        )]
    };
    warnings.extend(runtime.load_warnings.iter().cloned());
    if lm2_v20_runtime_preset_enabled() {
        warnings.push("LM2 v20 runtime preset is enabled.".to_owned());
    }
    if lm2_v25_d1_runtime_preset_enabled() {
        warnings.push("LM2 v25 D1 zero-spend runtime preset is enabled.".to_owned());
    }
    if (runtime.start_score_scale - 1.0).abs() > f64::EPSILON
        || (runtime.transition_score_scale - 1.0).abs() > f64::EPSILON
    {
        warnings.push(format!(
            "LM2 decoder start/transition scales are {:.3}/{:.3}.",
            runtime.start_score_scale, runtime.transition_score_scale
        ));
    }
    if let Some(label) = context_twopass_label.as_deref() {
        warnings.push(format!(
            "Review Mode learned postprocessor stack is active: {label}."
        ));
    }
    if !runtime.native_line_model_active()
        && runtime.numeric_catboost_model.is_none()
        && runtime.model.is_none()
    {
        warnings.push("LM2 trained model not found; used geometry fallback emissions.".to_owned());
    }
    if let Some(external) = external_emissions.as_ref() {
        warnings.push(format!(
            "LM2 external emissions override is enabled: {}. LM2 document cache was bypassed.",
            external.source_label()
        ));
    }
    if runtime.marker_decoder_prior {
        warnings.push("EXP-061 marker-continuity decoder prior is enabled.".to_owned());
    }
    if runtime.small_font_decoder_prior {
        warnings.push("EXP-062 small-font lower-page decoder prior is enabled.".to_owned());
    }
    if runtime.anchored_marginalia_flow_guard {
        warnings.push("EXP-069 anchored marginalia flow guard is enabled.".to_owned());
    }
    if runtime.body_preservation_guard {
        warnings.push("LM2 body-preservation guard is enabled.".to_owned());
    }
    if runtime.action_neutral_blocksplit {
        warnings.push("LM2 action-neutral blocksplit is enabled.".to_owned());
    }
    if runtime.toc_overlay {
        warnings.push("LM2 document-local TOC overlay is enabled.".to_owned());
    }
    if runtime.front_matter_guard {
        warnings.push("LM2 first-page front-matter guard is enabled.".to_owned());
    }
    if runtime.marginalia_preservation_guard {
        warnings.push("LM2 marginalia-preservation guard is enabled.".to_owned());
    }
    if d1_runtime_zerospend_overlay {
        warnings.push("LM2 D1 zero-spend keep-to-marginalia overlay is enabled.".to_owned());
    }
    if d1_runtime_continuation_overlay {
        warnings.push("LM2 D1 continuation keep-to-marginalia overlay is enabled.".to_owned());
    }
    if d1_runtime_immediate_continuation_overlay {
        warnings.push(
            "LM2 D1 immediate-neighbor continuation keep-to-marginalia overlay is enabled."
                .to_owned(),
        );
    }
    if d1_runtime_sandwiched_continuation_overlay {
        warnings.push(
            "LM2 D1 sandwiched continuation keep-to-marginalia overlay is enabled.".to_owned(),
        );
    }
    if d1_runtime_wide_sandwich_overlay {
        warnings.push(
            "LM2 D1 wide sandwiched continuation keep-to-marginalia overlay is enabled.".to_owned(),
        );
    }
    if d1_runtime_safe_numeric_note_overlay {
        warnings.push(
            "LM2 D1 safe numeric note-start keep-to-marginalia overlay is enabled.".to_owned(),
        );
    }
    if d1_runtime_post_wide_cue_overlay {
        warnings.push("LM2 D1 post-wide cue keep-to-marginalia overlay is enabled.".to_owned());
    }
    if d1_runtime_postcue_citation_next1_overlay {
        warnings.push(
            "LM2 D1 postcue citation-next1 keep-to-marginalia overlay is enabled.".to_owned(),
        );
    }
    if d1_runtime_near8_cue_overlay {
        warnings.push("LM2 D1 near8 cue keep-to-marginalia overlay is enabled.".to_owned());
    }
    if d1_runtime_wide_divider_guard_overlay {
        warnings
            .push("LM2 D1 wide-divider guarded keep-to-marginalia overlay is enabled.".to_owned());
    }
    if d1_runtime_footer_artifact_overlay {
        warnings.push("LM2 D1 footer-artifact hide-noise overlay is enabled.".to_owned());
    }
    if footnote_monotone_overlay {
        warnings.push("LM2 global footnote-number monotonicity overlay is enabled.".to_owned());
    }
    if footnote_carryover_overlay {
        warnings.push("LM2 footnote carryover hard overlay is enabled.".to_owned());
    }
    if open_footnote_carryover_overlay {
        warnings.push("LM2 open-footnote carryover hard overlay is enabled.".to_owned());
    }
    if page_object_tuned_overlay {
        warnings.push("LM2 page-object tuned hide-noise overlay is enabled.".to_owned());
    }
    if let Some(overlay) = runtime.static_front_overlay.as_ref() {
        warnings.push(format!(
            "LM2 static front overlay is enabled: {}.",
            overlay.source_label
        ));
    }
    if runtime.pp_footnote_region_membership {
        warnings.push("EXP-075 PP footnote-region membership override is enabled.".to_owned());
    }
    if article_spans.len() > 1 {
        warnings.push(format!(
            "Detected {} article spans in this PDF.",
            article_spans.len()
        ));
    }
    warnings.extend(pp_runtime_warnings);
    warnings.extend(liquidvision_runtime_warnings);
    if let Some(grouping) = &grouping {
        if let Some(mode) = &grouping.mode {
            warnings.push(format!("PyMuPDF block grouping mode: {mode}"));
        }
        warnings.extend(
            grouping
                .warnings
                .iter()
                .map(|warning| format!("PyMuPDF block grouping: {warning}")),
        );
    }

    let mut document = LiquidDocument {
        title,
        blocks,
        article_spans,
        block_source_lines,
        footnote_links: Vec::new(),
        footnote_link_integrity: None,
        profile: Some(lm2_profile()),
        noise_lines_removed: hidden_count,
        llm_used: false,
        llm_provider: Some("LM2".to_owned()),
        deep_liquid_used: false,
        deep_liquid_model: Some(runtime.model_label.clone()),
        warnings,
        source_signature,
    };
    attach_footnote_links(&mut document);
    if external_emissions.is_none() && !open_footnote_carryover_overlay {
        let _ = save_cached_lm2_document(&document);
    }
    timing.assembly_ms = assembly_started.elapsed().as_secs_f64() * 1000.0;
    timing.total_ms = total_started.elapsed().as_secs_f64() * 1000.0;
    Ok((document, timing))
}

pub(super) fn detect_lm2_article_spans(
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    page_count: usize,
) -> Vec<crate::liquid::ArticleSpan> {
    let lines = decoded
        .iter()
        .map(|(line, action)| ArticleSegmentationLine {
            page_index: line.page_index,
            line_index: line.line_index,
            text: line.text.clone(),
            font_ratio_page: line.font_ratio_page,
            font_ratio_doc: line.font_ratio_doc,
            margin_centered: line.margin_centered,
            line_width_ratio: line.line_width_ratio,
            top: line.top,
            page_height: line.page_height,
            repeated_edge_text: line.doc_repeated_edge_text,
            toc_like: line.segment_block_toc_like,
            note_marker: note_head_marker(&line.text)
                .or((line.doc_note_marker > 0).then_some(line.doc_note_marker)),
            // Boundary evidence must not depend solely on the final emission
            // action. The old scans that most need article scoping are also
            // the documents where note lines are commonly misclassified.
            marginalia: *action == Lm2Action::Marginalia
                || line.doc_footnote_state
                || line.in_footnote_zone
                || line.below_footnote_divider
                || line.segment_block_footnote_like,
        })
        .collect::<Vec<_>>();
    let detected = detect_article_spans(&lines, page_count);
    filter_lm2_false_article_spans(detected, decoded, page_count)
}

pub(super) fn filter_lm2_false_article_spans(
    detected: Vec<ArticleSpan>,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
    page_count: usize,
) -> Vec<ArticleSpan> {
    if detected.len() <= 1 {
        return detected;
    }
    let mut kept = detected
        .into_iter()
        .enumerate()
        .filter_map(|(index, span)| {
            (index == 0 || !lm2_false_article_boundary(&span, decoded)).then_some(span)
        })
        .collect::<Vec<_>>();
    for index in 0..kept.len() {
        kept[index].article_index = index;
        if let Some((next_page, next_line)) = kept
            .get(index + 1)
            .map(|next| (next.start_page_index, next.start_line_index))
        {
            kept[index].end_page_index = next_page;
            kept[index].end_line_index = next_line;
        } else {
            kept[index].end_page_index = page_count;
            kept[index].end_line_index = 0;
        }
    }
    kept
}

pub(super) fn lm2_false_article_boundary(
    span: &ArticleSpan,
    decoded: &[(DeepLiquidSourceLine, Lm2Action)],
) -> bool {
    if span.start_page_index == 0 {
        return false;
    }
    let caption_band = decoded.iter().any(|(line, _)| {
        line.page_index == span.start_page_index
            && line.line_index <= 8
            && lm2_numbered_table_figure_caption(&line.text)
    });
    if caption_band {
        return true;
    }

    let typography_only_internal_outline = span.confidence <= 0.60
        && span
            .title_hint
            .as_deref()
            .is_some_and(lm2_internal_outline_title_hint)
        && !span
            .evidence
            .iter()
            .any(|item| matches!(item.kind.as_str(), "publication_section" | "footnote_reset"));
    typography_only_internal_outline
}

pub(super) fn lm2_numbered_table_figure_caption(text: &str) -> bool {
    let normalized = normalize_text(text);
    let mut words = normalized.split_whitespace();
    if !matches!(words.next(), Some("table" | "figure" | "fig.")) {
        return false;
    }
    words.next().is_some_and(|token| {
        let marker = token.trim_matches(|ch: char| matches!(ch, '.' | ':' | ')' | ']'));
        !marker.is_empty()
            && marker.len() <= 8
            && (marker.chars().all(|ch| ch.is_ascii_digit())
                || marker.chars().all(|ch| {
                    matches!(
                        ch.to_ascii_lowercase(),
                        'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'
                    )
                }))
    })
}

pub(super) fn lm2_explicit_numbered_table_figure_caption(text: &str) -> bool {
    let normalized = normalize_text(text);
    let mut words = normalized.split_whitespace();
    if !matches!(words.next(), Some("table" | "figure" | "fig.")) {
        return false;
    }
    words.next().is_some_and(|token| {
        let has_caption_punctuation = token.ends_with(':');
        let marker = token.trim_end_matches(':');
        has_caption_punctuation
            && !marker.is_empty()
            && marker.len() <= 8
            && (marker.chars().all(|ch| ch.is_ascii_digit())
                || marker.chars().all(|ch| {
                    matches!(
                        ch.to_ascii_lowercase(),
                        'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'
                    )
                }))
    })
}

pub(super) fn lm2_internal_outline_title_hint(text: &str) -> bool {
    let token = text.split_whitespace().next().unwrap_or_default();
    if !token.ends_with('.') && !token.ends_with(')') {
        return false;
    }
    let marker = token.trim_matches(|ch: char| matches!(ch, '(' | ')' | '.'));
    !marker.is_empty()
        && marker.len() <= 8
        && marker.chars().all(|ch| {
            matches!(
                ch.to_ascii_lowercase(),
                'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'
            )
        })
}

pub(crate) fn trace_article_spans_from_source_lines(
    source_lines: &[DeepLiquidSourceLine],
    page_count: usize,
) -> (
    Vec<crate::liquid::ArticleSpan>,
    Vec<ArticleBoundaryCandidateTrace>,
) {
    let lines = source_lines
        .iter()
        .map(|line| ArticleSegmentationLine {
            page_index: line.page_index,
            line_index: line.line_index,
            text: line.text.clone(),
            font_ratio_page: line.font_ratio_page,
            font_ratio_doc: line.font_ratio_doc,
            margin_centered: line.margin_centered,
            line_width_ratio: line.line_width_ratio,
            top: line.top,
            page_height: line.page_height,
            repeated_edge_text: line.doc_repeated_edge_text,
            toc_like: line.segment_block_toc_like,
            note_marker: note_head_marker(&line.text)
                .or((line.doc_note_marker > 0).then_some(line.doc_note_marker)),
            marginalia: line.doc_footnote_state
                || line.in_footnote_zone
                || line.below_footnote_divider
                || line.segment_block_footnote_like,
        })
        .collect::<Vec<_>>();
    detect_article_spans_with_trace(&lines, page_count)
}

//! Move tool drags shown live (#128): while the pointer drags, the canvas shows the document with
//! the moving layers already at the pointer, exactly as `layer.translate` will leave them, and
//! recomposites only where they were and where they are. Releasing commits one `layer.translate`
//! (one history step) whose damage rect refreshes the same area.
//!
//! With a selection, the Move tool drags the selected pixels instead (#578): they float
//! (`select.float`, the canvas shows the piece at the pointer) until ↩, Deselect or any other
//! command drops them, as one "Move" history step.

use std::sync::Arc;

use photocraft_doc::{DocId, Document, LayerId};
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// Does the Move tool drag (and do the arrow keys nudge) the selected pixels of the active layer
/// rather than the whole layer, with `tool` in effect? With a selection, as in Photoshop; a layer
/// whose pixels can't float (type, shape, Smart Object, group, position-locked), a hidden layer or
/// several selected layers still move whole, as without one. The canvas cursor asks before the
/// press, and a pointer event passes `canvas::event_tool`, which also sees a ⌘ sent with the event
/// (#2768).
pub(crate) fn moves_selected_pixels_with(app: &PhotocraftApp, tool: Tool) -> bool {
    let Some(st) = app.session.active() else { return false };
    tool == Tool::Move
        && app.ui.transform.is_none()
        && st.doc.selection.is_some()
        && st.selected_layers().len() <= 1
        && st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| l.visible)
        && app.session.is_enabled("select.float")
}

/// Floats the selected pixels and moves them by (dx, dy) (`select.float`): cuts them the first
/// time, or with `copy` (⌥) floats a duplicate and leaves them; `copy` on a piece already floating
/// drops it where it is and floats a copy of it. False when it failed (reported in the status bar).
pub(crate) fn float_selected(app: &mut PhotocraftApp, copy: bool, dx: f64, dy: f64) -> bool {
    let floating = app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_some());
    let dropped = if copy && floating { app.run("select.drop", json!({})) } else { Ok(json!(null)) };
    match dropped.and_then(|_| app.run("select.float", json!({"dx": dx, "dy": dy, "copy": copy}))) {
        Ok(_) => {
            crate::snap_ui::follow_cut(app);
            true
        }
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            false
        }
    }
}

/// Preview keys of move drags: `BASE + n`, one per offset shown (see `canvas::display_doc`).
const BASE: u64 = 1 << 39;
/// Key of the document shown under a live drag of a floating piece: the cut-out layer without
/// the piece, which [`draw_content`] draws on its own at the pointer.
const OVERLAY: u64 = BASE - 1;

/// A floating piece drawn over the canvas while it's dragged: its texels, uploaded once, where it
/// was cut, and the layers covering it ([`crate::transform_tool::Above`]), drawn over it.
pub(crate) struct FloatOverlay {
    texture: crate::transform_tex::PreviewTextures,
    bounds: Rect,
    above: crate::transform_tool::Above,
}

pub(crate) struct MovePreview {
    doc: DocId,
    revision: u64,
    /// Layers that move ([`photocraft_engine::layer_multi_cmds::move_targets`]).
    ids: Vec<LayerId>,
    /// `ids` and every layer inside them: the layers shown at the drag's offset.
    moving: Vec<LayerId>,
    /// What moving them can change at offset (0, 0), not clipped to the canvas: a layer larger
    /// than the canvas brings its pixels from beyond the edge into view (`None` = anything).
    bounds: Option<Rect>,
    canvas: Rect,
    /// Offsets shown so far, by preview key (`BASE + index`).
    offsets: Vec<(i32, i32)>,
    /// The document at the latest offset.
    shown: Option<Arc<Document>>,
    /// Shows a floating selection (`select.float`): offsets are the piece's, from where it was cut.
    floating: bool,
    /// A live drag of a floating piece that draws it over the canvas instead of compositing it
    /// at each offset ([`overlay_doc`]): the document without the piece, built once per drag.
    base: Option<Arc<Document>>,
    /// Where `base` also differs from the document: the layers covering the piece, left out of
    /// it because they're drawn over the piece.
    above_area: Rect,
    /// The piece's textures for that drag, built on the first frame drawn ([`draw_content`]).
    overlay: Option<FloatOverlay>,
}

impl MovePreview {
    fn key(&self) -> u64 {
        BASE + self.offsets.len() as u64
    }
    fn offset_of(&self, key: u64) -> Option<(i32, i32)> {
        if key == 0 {
            return Some((0, 0));
        }
        let i = usize::try_from(key.checked_sub(BASE + 1)?).ok()?;
        self.offsets.get(i).copied()
    }
    fn area(&self, d: (i32, i32)) -> Option<Rect> {
        let b = self.bounds?;
        Some(if b.is_empty() { b } else { b.translate(d.0, d.1).inflate(1).intersect(&self.canvas) })
    }
}

/// The whole-pixel offset of the current Move drag on document `idx`, if one is under way.
fn drag_offset(app: &PhotocraftApp) -> Option<(i32, i32)> {
    let d = app.drag.as_ref().filter(|d| d.tool == Tool::Move)?;
    let end = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
    let dx = (end[0] - d.start[0]).round().clamp(-1e7, 1e7) as i32;
    let dy = (end[1] - d.start[1]).round().clamp(-1e7, 1e7) as i32;
    Some((dx, dy))
}

/// The ΔX/ΔY values shown beside the pointer while the Move tool drags layers or selected pixels,
/// as in Photoshop (#5727): the whole pixels the release will move them. Free Transform shows its
/// own (`transform_tool::live_readout`). `None` until the pointer moves.
pub(crate) fn offset_readout(app: &PhotocraftApp) -> Option<[String; 2]> {
    app.drag.as_ref().filter(|d| d.points.len() > 1)?;
    let (dx, dy) = drag_offset(app)?;
    Some([dx, dy].map(|v| format!("{v} px")))
}

/// The document to show while a Move drag is under way on document `idx`: the moving layers at
/// the pointer. `None` without a drag (or when the layers can't move: locked, say; the drag then
/// shows its arrow and the release reports why).
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    if app.session.active_index() == Some(idx)
        && let Some(st) = app.session.documents().get(idx)
        && photocraft_engine::float_cmds::floating(st).is_some()
    {
        return floating_doc(app, idx);
    }
    // Preferences › Interface › Show bounding box when dragging layer: outline and arrow only.
    if app.session.active_index() != Some(idx) || app.session.prefs().interface.show_bounding_box_when_dragging_layer {
        return None;
    }
    let Some(offset) = drag_offset(app) else {
        app.move_preview = None;
        return None;
    };
    let mask_target = app.layer_mask_targeted();
    let st = app.session.documents().get(idx)?;
    let (doc_id, revision, doc) = (st.doc.id, st.revision, st.doc.clone());
    let fresh = app.move_preview.as_ref().is_some_and(|p| p.doc == doc_id && p.revision == revision);
    if !fresh {
        let ids = photocraft_engine::layer_multi_cmds::move_targets(&doc, &st.selected_layers());
        if ids.is_empty() {
            return None;
        }
        let all = Rect::new(i32::MIN / 2, i32::MIN / 2, i32::MAX / 2, i32::MAX / 2);
        let bounds = ids.iter().try_fold(Rect::EMPTY, |acc, id| Some(acc.union(&photocraft_compose::change_bounds(doc.layer(*id)?, all)?)));
        let canvas = doc.bounds();
        let moving = with_descendants(&doc, &ids);
        let bounds = bounds.map(|b| moving.iter().filter_map(|id| shape_reach(doc.layer(*id)?, canvas)).fold(b, |acc, r| acc.union(&r)));
        app.move_preview =
            Some(MovePreview { doc: doc_id, revision, ids, moving, bounds, canvas, offsets: Vec::new(), shown: None, floating: false, base: None, above_area: Rect::EMPTY, overlay: None });
    }
    let p = app.move_preview.as_mut()?;
    if offset == (0, 0) && p.offsets.is_empty() {
        return None;
    }
    if p.offsets.last() != Some(&offset) {
        let t0 = crate::gpu_canvas::now_ms();
        match photocraft_engine::layer_multi_cmds::moved_target(&doc, &p.ids, offset.0, offset.1, mask_target) {
            Ok(d) => {
                // Duotone documents display through their inks.
                let d = photocraft_engine::mode_cmds::display_document(&d).unwrap_or(d);
                p.shown = Some(Arc::new(d));
                p.offsets.push(offset);
            }
            Err(_) => {
                p.shown = None;
                return None;
            }
        }
        app.perf.span("move preview", crate::gpu_canvas::now_ms() - t0);
    }
    let p = app.move_preview.as_ref()?;
    Some((p.shown.clone()?, p.key()))
}

/// Where shape layer `l` paints, effects included, not cut at the canvas as its rendered pixels
/// are: a drag brings the rest of it into view (#3946). `None` for other layers.
fn shape_reach(l: &photocraft_doc::Layer, canvas: Rect) -> Option<Rect> {
    let photocraft_doc::LayerContent::Shape(sh) = &l.content else { return None };
    let b = photocraft_vector::CompiledShape::new(sh, photocraft_vector::DEFAULT_TOLERANCE, canvas).bounds()?;
    let m = if photocraft_compose::effects::has_effects(l) { photocraft_compose::effects::margin(l) } else { 0 };
    Some(b.inflate(m))
}

/// `ids` and every layer inside them (groups move their contents).
fn with_descendants(doc: &Document, ids: &[LayerId]) -> Vec<LayerId> {
    let walk = doc.walk();
    let roots: Vec<_> = walk.iter().filter(|(_, _, l)| ids.contains(&l.id)).map(|(p, _, _)| p.as_slice()).collect();
    walk.iter().filter(|(p, _, _)| roots.iter().any(|r| p.starts_with(r))).map(|(_, _, l)| l.id).collect()
}

/// Where the canvas shows layer `id` of document `doc` at `revision` relative to where it is: the
/// offset of the latest Move drag frame drawn ([`display_doc`]) when the layer moves with the drag,
/// so its Layer Edges outline moves in lockstep with its pixels (#2413). `None` when it shows
/// where it is.
pub(crate) fn shown_layer_offset(app: &PhotocraftApp, doc: DocId, revision: u64, id: LayerId) -> Option<(i32, i32)> {
    let p = app.move_preview.as_ref().filter(|p| p.doc == doc && p.revision == revision && !p.floating && p.shown.is_some())?;
    if !p.moving.contains(&id) {
        return None;
    }
    p.offsets.last().copied()
}

/// A floating selection's offset plus a drag of it in progress (`canvas::selection_drag_delta`).
pub(crate) fn floating_offset(app: &PhotocraftApp) -> Option<(i32, i32)> {
    let f = photocraft_engine::float_cmds::floating(app.session.active()?)?;
    let d = crate::canvas::selection_drag_delta(app).unwrap_or((0, 0));
    Some((f.offset.0 + d.0, f.offset.1 + d.1))
}

/// The document shown while a selection floats: the cut piece at its offset, over the cut-out
/// layer. Only the piece's area changes between offsets, so each move costs the piece, not the
/// layer.
fn floating_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let offset = floating_offset(app)?;
    let st = app.session.documents().get(idx)?;
    let f = photocraft_engine::float_cmds::floating(st)?;
    let (doc_id, revision) = (st.doc.id, st.revision);
    let fresh = app.move_preview.as_ref().is_some_and(|p| p.doc == doc_id && p.revision == revision && p.floating);
    if !fresh {
        let bounds = st.doc.selection.as_ref().map(|s| s.content_bounds());
        let canvas = st.doc.bounds();
        app.move_preview = Some(MovePreview {
            doc: doc_id,
            revision,
            ids: vec![f.layer],
            moving: Vec::new(),
            bounds,
            canvas,
            offsets: Vec::new(),
            shown: None,
            floating: true,
            base: None,
            above_area: Rect::EMPTY,
            overlay: None,
        });
    }
    // While the piece is dragged, the canvas shows the document without it (composited once) and
    // draws the piece over it at the pointer: no pixels move until the drag ends. It starts once
    // the piece has moved (a press alone changes nothing) and lasts until the drag ends.
    let started = app.move_preview.as_ref().is_some_and(|p| p.floating && p.base.is_some());
    if crate::canvas::selection_drag_delta(app).is_some() && (started || offset != (0, 0)) {
        if let Some(shown) = overlay_doc(app, idx) {
            return Some(shown);
        }
    } else if let Some(p) = app.move_preview.as_mut() {
        p.base = None;
        p.overlay = None;
    }
    if offset == (0, 0) && app.move_preview.as_ref().is_none_or(|p| p.offsets.is_empty()) {
        return None;
    }
    let st = app.session.documents().get(idx)?;
    let shown = app.move_preview.as_ref()?.offsets.last() != Some(&offset);
    if shown {
        let t0 = crate::gpu_canvas::now_ms();
        let d = photocraft_engine::float_cmds::displayed(st, crate::canvas::selection_drag_delta(app).unwrap_or((0, 0)))?;
        let d = photocraft_engine::mode_cmds::display_document(&d).unwrap_or(d);
        let p = app.move_preview.as_mut()?;
        p.shown = Some(Arc::new(d));
        p.offsets.push(offset);
        app.perf.span("floating preview", crate::gpu_canvas::now_ms() - t0);
    }
    let p = app.move_preview.as_ref()?;
    Some((p.shown.clone()?, p.key()))
}

/// Whether a floating piece on layer `id` of `doc` can be drawn over the canvas on its own while
/// it's dragged and look exactly as composited: pixels (not a mask) of a visible Normal layer at
/// full opacity, with no style, mask or layers clipped to it, in Normal or Pass Through groups at
/// full opacity, in an 8-bit RGB or Grayscale document. Anything else keeps compositing each
/// offset.
fn overlay_fits(doc: &Document, id: LayerId, parts: &photocraft_engine::float_cmds::CutParts) -> bool {
    use photocraft_color::{BlendMode, ColorMode, SampleType};
    fn plain(l: &photocraft_doc::Layer) -> bool {
        l.visible
            && l.opacity >= 1.0
            && !photocraft_compose::effects::has_effects(l)
            && l.mask.as_ref().is_none_or(|m| !m.enabled)
            && l.vector_mask.as_ref().is_none_or(|v| !v.enabled)
    }
    /// `Some(true)` when `id` is in `layers` and fits there (and in the groups above it).
    fn fits(layers: &[photocraft_doc::Layer], id: LayerId, depth: usize) -> Option<bool> {
        if depth > 64 {
            return Some(false);
        }
        for (i, l) in layers.iter().enumerate() {
            let clipped_to = layers.get(i + 1).is_some_and(|n| n.clipped);
            if l.id == id {
                return Some(plain(l) && l.blend == BlendMode::Normal && l.fill_opacity >= 1.0 && !l.clipped && !clipped_to);
            }
            if let photocraft_doc::LayerContent::Group(g) = &l.content
                && let Some(inside) = fits(&g.children, id, depth + 1)
            {
                return Some(inside && plain(l) && matches!(l.blend, BlendMode::Normal | BlendMode::PassThrough) && !l.clipped && !clipped_to);
            }
        }
        None
    }
    !parts.is_mask()
        && matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale)
        && doc.depth == SampleType::U8
        && !parts.piece().1.is_empty()
        && fits(&doc.layers, id, 0).unwrap_or(false)
}

/// The document shown under a live drag of the floating piece on document `idx`, when the piece
/// can be drawn over it ([`overlay_fits`]): its layer cut out, the layers covering it left out
/// (they're drawn over the piece), built once per drag.
fn overlay_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    if app.move_preview.as_ref().is_some_and(|p| p.floating && p.base.is_some()) {
        let p = app.move_preview.as_ref()?;
        return Some((p.base.clone()?, OVERLAY));
    }
    let st = app.session.documents().get(idx)?;
    let f = photocraft_engine::float_cmds::floating(st)?;
    if !overlay_fits(&st.doc, f.layer, &f.parts) {
        return None;
    }
    let t0 = crate::gpu_canvas::now_ms();
    let mut base = f.parts.without_piece(&st.doc).ok()?;
    let mut above = Vec::new();
    crate::transform_tool::covering(&st.doc.layers, f.layer, &mut above);
    hide(&mut base.layers, &above);
    let canvas = st.doc.bounds();
    let above_area = above
        .iter()
        .filter_map(|id| st.doc.layer(*id))
        .fold(Rect::EMPTY, |r, l| r.union(&photocraft_compose::composite_bounds(l, canvas).unwrap_or(canvas)))
        .intersect(&canvas);
    let base = Arc::new(base);
    let p = app.move_preview.as_mut().filter(|p| p.floating)?;
    p.base = Some(base.clone());
    p.above_area = above_area;
    p.overlay = None;
    app.perf.span("floating overlay", crate::gpu_canvas::now_ms() - t0);
    Some((base, OVERLAY))
}

/// Hide the layers `ids` (at any depth) of `layers`.
fn hide(layers: &mut [photocraft_doc::Layer], ids: &[LayerId]) {
    for l in layers.iter_mut() {
        if ids.contains(&l.id) {
            l.visible = false;
        } else if let photocraft_doc::LayerContent::Group(g) = &mut l.content {
            hide(&mut g.children, ids);
        }
    }
}

/// Draw a floating piece being dragged over the canvas at the pointer ([`overlay_doc`]), with the
/// layers covering it over it. Its textures are built on the first frame and reused after.
pub(crate) fn draw_content(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &crate::canvas::ViewXform) {
    if !app.move_preview.as_ref().is_some_and(|p| p.floating && p.base.is_some()) || crate::canvas::selection_drag_delta(app).is_none() {
        return;
    }
    let Some(offset) = floating_offset(app) else { return };
    let Some(st) = app.session.active() else { return };
    let Some(f) = photocraft_engine::float_cmds::floating(st) else { return };
    let (doc, layer, parts) = (st.doc.clone(), f.layer, f.parts.clone());
    let ctx = painter.ctx().clone();
    let size = [doc.size.width as f32, doc.size.height as f32];
    if app.move_preview.as_ref().is_some_and(|p| p.overlay.is_none()) {
        let t0 = crate::gpu_canvas::now_ms();
        let max_side = ctx.input(|i| i.max_texture_side).max(app.gpu.as_ref().map_or(1, |g| g.max_texture_side()));
        let (piece, bounds) = parts.piece();
        let (image, uv) = crate::transform_tex::read_surface(piece, None, bounds, max_side);
        let texture = crate::transform_tex::PreviewTextures::new(&ctx, "floating-piece".into(), image, uv);
        let above = crate::transform_tool::Above::new(&ctx, "floating-above".into(), &doc, layer, max_side);
        if let Some(p) = app.move_preview.as_mut() {
            p.overlay = Some(FloatOverlay { texture, bounds, above });
        }
        app.perf.span("floating overlay textures", crate::gpu_canvas::now_ms() - t0);
    }
    let output = crate::monitor_status::view_display(app, &ctx);
    let (display, _) = crate::canvas::canvas_display(app, &doc, output);
    let Some(o) = app.move_preview.as_ref().and_then(|p| p.overlay.as_ref()) else { return };
    let (tex, uv) = o.texture.pick_display(&ctx, xf.zoom * ctx.pixels_per_point(), 1.0, false, display.as_deref());
    let r = o.bounds.translate(offset.0, offset.1);
    let mut mesh = egui::Mesh::with_texture(tex);
    for (x, y, u, v) in [(r.x0, r.y0, 0.0, 0.0), (r.x1, r.y0, uv[0], 0.0), (r.x1, r.y1, uv[0], uv[1]), (r.x0, r.y1, 0.0, uv[1])] {
        mesh.vertices.push(egui::epaint::Vertex { pos: egui::pos2(x as f32, y as f32), uv: egui::pos2(u, v), color: egui::Color32::WHITE });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    // Pixels moved past the canvas edge are cut there, as the move will leave them.
    painter.add(crate::transform_tool::clip_to_canvas(mesh, size, |p| xf.to_screen(p.x, p.y)));
    o.above.draw(painter, xf, display.as_deref());
}

/// What changed between preview (or document) key `seen` and `now` of the current Move drag on
/// document `doc` at `revision`: where the moving layers were in both. `None` when either key
/// isn't one of this drag's (the canvas then recomposites everything).
pub(crate) fn damage(app: &PhotocraftApp, doc: DocId, revision: u64, seen: u64, now: u64) -> Option<Rect> {
    let p = app.move_preview.as_ref().filter(|p| p.doc == doc && p.revision == revision)?;
    let ours = |k: u64| k == 0 || k >= BASE || (k == OVERLAY && p.floating);
    if seen == now || !ours(seen) || !ours(now) {
        return None;
    }
    // The document under a live drag differs from the others only where they show the piece.
    let area = |k: u64| if k == OVERLAY { Some(p.above_area) } else { p.area(p.offset_of(k)?) };
    let (a, b) = (area(seen)?, area(now)?);
    // Going to or from the document without the piece also changes where the covering layers are.
    let (a, b) = if seen == OVERLAY || now == OVERLAY { (a.union(&p.above_area), b.union(&p.above_area)) } else { (a, b) };
    Some(if a.is_empty() {
        b
    } else if b.is_empty() {
        a
    } else {
        a.union(&b)
    })
}

/// Whether the canvas shows a Move drag's layers at the pointer.
pub(crate) fn showing(app: &PhotocraftApp) -> bool {
    app.move_preview.as_ref().is_some_and(|p| p.shown.is_some())
}

/// Whether `key` is a Move drag preview key.
pub(crate) fn is_preview_key(key: u64) -> bool {
    key > BASE && key < BASE << 1
}

/// Does a Move drag of whole layers take the selection along? With several layers selected, as
/// in Photoshop (#7538): their selected pixels can't float, so the layers move and the marching
/// ants follow them.
pub(crate) fn carries_selection(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|st| st.doc.selection.is_some() && st.selected_layers().len() > 1)
}

/// Release: one `layer.translate` by the drag's offset. The canvas already shows the result, so
/// its caches are told it showed the document: the command's damage rect refreshes that area.
pub(crate) fn finish(app: &mut PhotocraftApp, dx: f64, dy: f64) {
    // Within the range the preview and the ΔX/ΔY readout show (`drag_offset`), so the release
    // commits the offset they showed even at the lowest zoom of a huge document.
    let (dx, dy) = (dx.clamp(-1e7, 1e7), dy.clamp(-1e7, 1e7));
    // Only when the canvas shows this very offset (else it recomposites everything once).
    let at = (dx as i32, dy as i32);
    let shown = app.move_preview.take().filter(|p| p.shown.is_some() && p.offsets.last() == Some(&at));
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    if app.run("layer.translate", json!({"dx": dx, "dy": dy, "selection": carries_selection(app)})).is_err() {
        return;
    }
    let Some(p) = shown else { return };
    crate::canvas::shown_as_document(app, p.doc, is_preview_key);
}

#[cfg(test)]
mod tests;

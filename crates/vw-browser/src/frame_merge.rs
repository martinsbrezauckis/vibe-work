//! Combines validated frame observations and transforms their geometry into one page model.
//! Depends on the frame reader's document and origin facts; called only by that reader.
use super::{
    Ancestor, FrameRef, Frames, Kind, Observation, Omitted, UNCHECKED, UNREADABLE, frame_choices,
    pos,
};
use crate::model::PageModel;

/// Gives each inherited child (sorted by path, parents first) its parent's verified origin when its own security
/// origin is that one; else `null` (opaque: a sandboxed frame). A child whose parent was left out is left out too, and
/// a rewritten `about:blank` frame whose address is not on that origin is left out as unchecked.
pub(super) fn inherit(
    frames: Vec<(String, bool, Observation, Kind, Option<String>)>,
    unread: &mut Vec<Omitted>,
) -> Frames {
    let mut out: Frames = Vec::with_capacity(frames.len());
    for (id, main, mut observation, kind, reported) in frames {
        let has_parent = |path: &[usize]| {
            path.split_last()
                .is_some_and(|(_, parent)| out.iter().any(|(_, _, o)| o.path == parent))
        };
        if kind == Kind::Opaque && !has_parent(&observation.path) {
            continue;
        }
        if kind == Kind::Inherited {
            let parent = observation.path.split_last().and_then(|(_, path)| {
                out.iter()
                    .find(|(_, _, o)| o.path == path)
                    .map(|(_, _, o)| o.origin.clone())
            });
            let Some(parent) = parent else {
                continue;
            };
            let own = std::mem::take(&mut observation.origin);
            let proven = own != "null" && own == parent;
            if reported.is_some() && (!proven || reported.as_deref() != Some(own.as_str())) {
                unread.push((UNCHECKED, Some(observation.path)));
                continue;
            }
            observation.origin = if proven { own } else { "null".into() };
        }
        // Several handles of one frame (old handles kept by `WebKit` on macOS) answer from the same document: it is
        // read once. Two different documents claiming one frame position are left out, as unchecked.
        if let Some(index) = out.iter().position(|(_, _, o)| o.path == observation.path) {
            if out[index].2.document != observation.document {
                out[index].2.document.clear();
            }
            continue;
        }
        out.push((id, main, observation));
    }
    unread.extend(
        out.iter()
            .filter(|(_, _, o)| o.document.is_empty())
            .map(|(_, _, o)| (UNCHECKED, Some(o.path.clone()))),
    );
    out.retain(|(_, _, o)| !o.document.is_empty());
    out
}

/// Whether the read frames hold a frame this observation neither reads nor accounts for: a child window of a read
/// frame with no read frame and no reason of its own at its path (on Linux, one sandboxed without scripts from its
/// creation never reaches the engine's frame registry), or any frame in an open shadow root (no child window, so
/// never read, whether or not a handle answered). Such a frame names the page's frames unreadable, whatever
/// reasons other frames gave.
fn left_out(frames: &[(String, bool, Observation)], unread: &[Omitted]) -> bool {
    frames.iter().any(|(_, _, o)| o.shadowed > 0)
        || frames
            .iter()
            .flat_map(|(_, _, o)| {
                (0..o.windows.min(64)).map(|index| {
                    let mut path = o.path.clone();
                    path.push(index);
                    path
                })
            })
            .any(|path| {
                !frames.iter().any(|(_, _, o)| o.path == path)
                    && !unread.iter().any(|(_, p)| p.as_ref() == Some(&path))
            })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn merge(
    frames: &[(String, bool, Observation)],
    unread: Vec<Omitted>,
) -> Result<PageModel, String> {
    let top = frames
        .iter()
        .position(|(_, main, o)| *main && o.path.is_empty())
        .ok_or("the main frame changed while it was read")?;
    let mut model = frames[top].2.model.clone();
    model.document.clone_from(&frames[top].2.document);
    model.text_page_count = Some(
        model
            .text_length
            .div_ceil(crate::observe::PAGE_CHARS as u64)
            .max(1),
    );
    model.viewport = Some((
        frames[top].2.width.round() as u32,
        frames[top].2.height.round() as u32,
    ));
    reset_observed_content(&mut model);
    model.cross_origin_frames = 0;
    model.unread_frames.clear();
    let left_out = left_out(frames, &unread);
    let mut unread: Vec<_> = unread.into_iter().map(|(reason, _)| reason).collect();
    if left_out {
        unread.push(UNREADABLE);
    }
    for reason in unread {
        if !model.unread_frames.iter().any(|r| r == reason) {
            model.unread_frames.push(reason.to_owned());
        }
    }
    for (id, main, observation) in frames {
        let Some((x, y, sx, sy)) = transform(frames, &observation.path) else {
            continue;
        };
        let frame = FrameRef {
            ancestors: ancestry(frames, &observation.path).ok_or("frame ancestry changed")?,
            id: id.clone(),
            document: observation.document.clone(),
            origin: observation.origin.clone(),
            url: observation.model.url.clone(),
        };
        model.documents.push(frame.clone());
        frame_choices::append_main(&mut model, observation, &frame, *main);
        let visible = observation.path.iter().enumerate().all(|(depth, index)| {
            frames
                .iter()
                .find(|(_, _, o)| o.path == observation.path[..depth])
                .and_then(|(_, _, o)| o.children.iter().find(|c| c.index == pos(*index)))
                .is_some_and(|c| c.visible)
        });
        let prominent = *main || (visible && covers_viewport(frames, &observation.path));
        let mut choices = Vec::new();
        for (item, choice) in observation.model.refs.iter().map(|r| (r, false)).chain(
            observation
                .model
                .choice_refs
                .iter()
                .filter(|_| !main && prominent)
                .map(|r| (r, true)),
        ) {
            let mut item = item.clone();
            item.local_bounds = item.bounds;
            item.local_viewport = (observation.width as u32, observation.height as u32);
            item.frame_id = Some(frame.clone());
            if !main {
                item.frame = Some(true);
            }
            if !visible {
                item.visible = Some(false);
                item.priority = -1;
            } else if !prominent {
                item.priority = item.priority.min(3);
            }
            if let Some(rect) = &mut item.bounds {
                rect.x = (x + f64::from(rect.x) * sx).round() as i32;
                rect.y = (y + f64::from(rect.y) * sy).round() as i32;
                rect.width = (f64::from(rect.width) * sx).round() as i32;
                rect.height = (f64::from(rect.height) * sy).round() as i32;
            }
            if choice {
                choices.push(item);
            } else {
                model.refs.push(item);
            }
        }
        if !main && prominent {
            frame_choices::append_child(&mut model, observation, choices);
        }
        append_frame_content(&mut model, &observation.model, *main, visible);
    }
    if !crate::choices::unique(&model) {
        return Err(
            "frame refs collided, so this read's refs cannot be used; read the page again".into(),
        );
    }
    Ok(model)
}

fn reset_observed_content(model: &mut PageModel) {
    model.refs.clear();
    model.datasets.clear();
    model.choice_refs.clear();
    model.covering_frames.clear();
    model.all_refs.clear();
    model.ref_count = 0;
    model.selector_matches = model.selector_matches.map(|_| 0);
    model.scope_matches = 0;
}

fn append_frame_content(model: &mut PageModel, child: &PageModel, main: bool, visible: bool) {
    model.scope_matches = model.scope_matches.saturating_add(child.scope_matches);
    model
        .all_refs
        .extend(child.refs.iter().map(|r| r.id.clone()));
    model.ref_count = model.ref_count.saturating_add(child.ref_count);
    if visible {
        model.datasets.extend(child.datasets.iter().cloned());
    }
    if child.extraction_error.is_some() {
        model.extraction_error = Some("invalid selector".into());
    }
    if let Some(count) = child.selector_matches {
        model.selector_matches = Some(model.selector_matches.unwrap_or(0).saturating_add(count));
    }
    if child.selector_error.is_some() {
        model.selector_error = Some("invalid".into());
    }
    if !main {
        if visible {
            model.text_page_count = Some(
                model.text_page_count.unwrap_or(1).max(
                    child
                        .text_length
                        .div_ceil(crate::observe::PAGE_CHARS as u64),
                ),
            );
        }
        let text = if visible {
            &mut model.text
        } else {
            &mut model.hidden_text
        };
        if !child.text.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&child.text);
        }
        model.hidden_text.push('\n');
        model.hidden_text.push_str(&child.hidden_text);
        model.text_length = model.text_length.saturating_add(child.text_length);
    }
    model.truncated |= child.truncated;
}

/// Only a frame covering at least 40% of the top viewport can promote its overlays above main
/// form controls. This admits consent walls while excluding ordinary ads and banners. All boxes
/// come from parents, clipped through every ancestor: a child cannot enlarge its own allocation.
fn covers_viewport(frames: &[(String, bool, Observation)], path: &[usize]) -> bool {
    let Some((_, _, top)) = frames
        .iter()
        .find(|(_, main, o)| *main && o.path.is_empty())
    else {
        return false;
    };
    let area = top.width * top.height;
    if !area.is_finite() || area <= 0.0 {
        return false;
    }
    let (mut left, mut upper, mut right, mut bottom) = (0.0_f64, 0.0_f64, top.width, top.height);
    for (depth, index) in path.iter().enumerate() {
        let Some((x, y, sx, sy)) = transform(frames, &path[..depth]) else {
            return false;
        };
        let Some(child) = frames
            .iter()
            .find(|(_, _, o)| o.path == path[..depth])
            .and_then(|(_, _, o)| o.children.iter().find(|c| c.index == pos(*index)))
        else {
            return false;
        };
        let rect = child.content;
        let x = x + f64::from(rect.x) * sx;
        let y = y + f64::from(rect.y) * sy;
        left = left.max(x);
        upper = upper.max(y);
        right = right.min(x + f64::from(rect.width) * sx);
        bottom = bottom.min(y + f64::from(rect.height) * sy);
    }
    (right - left).max(0.0) * (bottom - upper).max(0.0) >= area * 0.4
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn ancestry(frames: &[(String, bool, Observation)], path: &[usize]) -> Option<Vec<Ancestor>> {
    path.iter()
        .enumerate()
        .map(|(depth, index)| {
            let (id, _, parent) = frames.iter().find(|(_, _, o)| o.path == path[..depth])?;
            let child = parent
                .children
                .iter()
                .find(|child| child.index == pos(*index))?;
            let (_, _, inner) = frames.iter().find(|(_, _, o)| o.path == path[..=depth])?;
            Some(Ancestor {
                id: id.clone(),
                document: parent.document.clone(),
                url: parent.model.url.clone(),
                node: child.node,
                bounds: child.bounds,
                content: child.content,
                viewport: (parent.width as u32, parent.height as u32),
                child_viewport: (inner.width as u32, inner.height as u32),
            })
        })
        .collect()
}

fn transform(
    frames: &[(String, bool, Observation)],
    path: &[usize],
) -> Option<(f64, f64, f64, f64)> {
    let (mut x, mut y, mut sx, mut sy) = (0.0, 0.0, 1.0, 1.0);
    for (depth, index) in path.iter().enumerate() {
        let parent = &frames.iter().find(|(_, _, o)| o.path == path[..depth])?.2;
        let child = parent
            .children
            .iter()
            .find(|child| child.index == pos(*index))?;
        if [child.x, child.y, child.sx, child.sy]
            .iter()
            .any(|n| !n.is_finite())
            || child.sx <= 0.0
            || child.sy <= 0.0
        {
            return None;
        }
        x += child.x * sx;
        y += child.y * sy;
        sx *= child.sx;
        sy *= child.sy;
    }
    Some((x, y, sx, sy))
}

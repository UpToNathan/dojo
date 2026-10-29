//! Presence nobody would see. A removed node that drew nothing last frame has no exit to show: it
//! faded out already, sits under a parent that hid at once, or lies out of the window or its
//! clips with everything it holds (a filtered list's rows scrolled out of view). It goes at
//! once rather than fading where nothing shows. Likewise a virtual row that a change mounted with
//! its entrance but that landed out of view lands its entrance at once, and one nobody saw that
//! a filter brings into view enters like the rows new to it.
use super::*;

impl<S: 'static> Runtime<S> {
    /// Per node of the last build: neither it nor anything under it drew. Empty before the first
    /// build and after a commit, until the next one draws.
    pub(super) fn drew_nothing(&self) -> Vec<bool> {
        if !self.built || self.seen.len() != self.nodes.len() || self.inh_scratch.len() != self.nodes.len() {
            return Vec::new();
        }
        let mut seen = self.seen.clone();
        mark_ancestors(&self.nodes, &mut seen);
        seen.iter().zip(&self.inh_scratch).map(|(&seen, h)| !seen || h.opacity < 1.0 / 512.0).collect()
    }

    /// Ends the exits that drew nothing last frame. A flight paints outside its clips, so it stays.
    pub(super) fn end_unseen_exits(&mut self) {
        if !self.built { return; }
        let mut hid: Option<Vec<bool>> = None;
        for i in 0..self.nodes.len() {
            let id = self.nodes[i].id;
            let faded = self.inh_scratch.get(i).is_some_and(|h| h.opacity < 1.0 / 512.0);
            if (!faded && self.seen.get(i).is_none_or(|&s| s)) || !self.is_exiting(id) || self.flying.iter().any(|f| f.0 == id) {
                continue;
            }
            if hid.get_or_insert_with(|| self.drew_nothing()).get(i).copied().unwrap_or(false) {
                self.scene.unmount(id);
            }
        }
    }

    /// Row `i` of the commit's `nodes`, kept by a change that replaced what the reader saw
    /// ([`crate::virtual_list::VirtualList::replaced_view`]) but drawn nowhere before, enters
    /// like the rows new to the view instead of appearing whole.
    pub(super) fn reenter_unseen_row(&mut self, nodes: &[Node], i: usize, unseen: &HashSet<NodeId>) {
        if !unseen.contains(&nodes[i].id) { return; }
        let Some(p) = nodes[i].parent else { return };
        let Some(v) = self.virtuals.get_mut(&nodes[p].path) else { return };
        let Some(key) = nodes[i].path.strip_prefix(nodes[p].path.as_str()).and_then(|k| k.strip_prefix('/')) else { return };
        if !v.replaced_view() { return; }
        v.entering.insert(key.to_owned());
        self.scene.reenter(nodes[i].id);
    }

    /// Virtual rows mounted with their entrance have just drawn once: the ones out of view land.
    /// Its own fade starts a row at no opacity, so only where it drew counts, strictly: a row
    /// touching its viewport's edge shows nothing. Runs after each draw, since a list filling its
    /// window commits again on the frames that follow.
    pub(super) fn land_unseen_entrances(&mut self) {
        if self.virtuals.values().all(|v| v.entering.is_empty()) { return; }
        let s = self.env.scale;
        let (w, h) = self.viewport;
        let mut land = Vec::new();
        for (path, v) in &mut self.virtuals {
            for key in v.entering.drain() {
                let Some(&id) = self.path_ids.get(&format!("{path}/{key}")) else { continue };
                let Some(&i) = self.node_index.get(&id) else { continue };
                let Some(&[x, y, rw, rh]) = self.drawn.get(i) else { continue };
                let clip = screen_clip_of(self.nodes[i].parent, &self.inh_scratch, &self.turn_scratch);
                let shows = (x + rw).min(clip[2] / s).min(w) > x.max(clip[0] / s).max(0.0)
                    && (y + rh).min(clip[3] / s).min(h) > y.max(clip[1] / s).max(0.0);
                if !shows { land.push(id); }
            }
        }
        for id in land { self.scene.enter_now(id); }
    }
}

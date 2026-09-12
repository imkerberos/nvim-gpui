use super::NvimGpui;
use gpui::Context;
use std::collections::{HashMap, HashSet};

impl NvimGpui {
    pub(super) fn schedule_multicursor_namespace_query(&mut self, cx: &mut Context<Self>) {
        if self.editor.cursor.multicursor_namespace_task.is_some()
            || self
                .editor
                .protocol
                .cursor
                .unresolved_multicursor_positions
                .is_empty()
        {
            return;
        }

        let Some(nvim) = self.app.session.nvim.as_ref() else {
            return;
        };
        let session_id = nvim.session_id();
        let response = match nvim.request("nvim_get_namespaces", rmpv::Value::Array(Vec::new())) {
            Ok(response) => response,
            Err(error) => {
                log::warn!(
                    target: "nvim_gpui::multicursor",
                    "could not query Neovim namespaces: {error}"
                );
                return;
            }
        };

        self.editor.cursor.multicursor_namespace_task = Some(cx.spawn(async move |weak, cx| {
            let result = response.recv().await;
            let result = match result {
                Ok(Ok(value)) => parse_multicursor_namespaces(&value),
                Ok(Err(error)) => Err(error),
                Err(error) => Err(format!("namespace response channel closed: {error}")),
            };
            let _ = weak.update(cx, |this, cx| {
                if this.app.session.session_id != Some(session_id) {
                    return;
                }
                this.editor.cursor.multicursor_namespace_task = None;
                match result {
                    Ok(namespaces) => this.install_multicursor_namespaces(namespaces),
                    Err(error) => log::warn!(
                        target: "nvim_gpui::multicursor",
                        "could not decode Neovim namespaces: {error}"
                    ),
                }
                cx.notify();
            });
        }));
    }

    fn install_multicursor_namespaces(&mut self, namespaces: HashMap<String, u64>) {
        self.editor.protocol.cursor.multicursor_namespace_ids = namespaces;
        let known_ids = self
            .editor
            .protocol
            .cursor
            .multicursor_namespace_ids
            .values()
            .copied()
            .collect::<HashSet<_>>();
        let unresolved =
            std::mem::take(&mut self.editor.protocol.cursor.unresolved_multicursor_positions);
        for (key, position) in unresolved {
            if known_ids.contains(&key.ns_id) {
                self.editor
                    .protocol
                    .cursor
                    .multicursor_positions
                    .insert(key, position);
                if let Some(pending) = self
                    .editor
                    .protocol
                    .cursor
                    .pending_multicursor_positions
                    .as_mut()
                {
                    pending.insert(key, position);
                }
            }
        }
    }

    pub(super) fn schedule_multicursor_reconcile(&mut self, cx: &mut Context<Self>) {
        if self.editor.cursor.multicursor_reconcile_task.is_some() {
            self.editor.cursor.multicursor_reconcile_dirty = true;
            return;
        }
        if self.editor.protocol.cursor.multicursor_positions.is_empty() {
            return;
        }

        let Some(nvim) = self.app.session.nvim.as_ref() else {
            return;
        };
        let session_id = nvim.session_id();
        let namespaces = self
            .editor
            .protocol
            .cursor
            .multicursor_namespace_ids
            .values()
            .copied()
            .collect::<HashSet<_>>();
        let mut responses = Vec::new();
        for ns_id in namespaces.iter().copied() {
            let params = rmpv::Value::Array(vec![
                rmpv::Value::from(0_i64),
                rmpv::Value::from(ns_id),
                rmpv::Value::Array(vec![rmpv::Value::from(0_i64), rmpv::Value::from(0_i64)]),
                rmpv::Value::Array(vec![rmpv::Value::from(-1_i64), rmpv::Value::from(-1_i64)]),
                rmpv::Value::Map(Vec::new()),
            ]);
            match nvim.request("nvim_buf_get_extmarks", params) {
                Ok(response) => responses.push((ns_id, response)),
                Err(error) => log::warn!(
                    target: "nvim_gpui::multicursor",
                    "could not query multicursor extmarks: {error}"
                ),
            }
        }
        if responses.is_empty() {
            return;
        }

        self.editor.cursor.multicursor_reconcile_task = Some(cx.spawn(async move |weak, cx| {
            let mut live_marks = HashMap::<u64, HashSet<u64>>::new();
            for (ns_id, response) in responses {
                let Ok(Ok(value)) = response.recv().await else {
                    continue;
                };
                if let Ok(mark_ids) = parse_extmark_ids(&value) {
                    live_marks.insert(ns_id, mark_ids);
                }
            }
            let _ = weak.update(cx, |this, cx| {
                if this.app.session.session_id != Some(session_id) {
                    return;
                }
                this.editor.cursor.multicursor_reconcile_task = None;
                this.reconcile_multicursor_marks(&live_marks);
                let rerun = std::mem::take(&mut this.editor.cursor.multicursor_reconcile_dirty);
                if rerun {
                    this.schedule_multicursor_reconcile(cx);
                }
                cx.notify();
            });
        }));
    }

    fn reconcile_multicursor_marks(&mut self, live_marks: &HashMap<u64, HashSet<u64>>) {
        let known_ids = self
            .editor
            .protocol
            .cursor
            .multicursor_namespace_ids
            .values()
            .copied()
            .collect::<HashSet<_>>();
        self.editor
            .protocol
            .cursor
            .multicursor_positions
            .retain(|key, _| {
                !known_ids.contains(&key.ns_id)
                    || live_marks
                        .get(&key.ns_id)
                        .map(|mark_ids| mark_ids.contains(&key.mark_id))
                        .unwrap_or(true)
            });
        if let Some(pending) = self
            .editor
            .protocol
            .cursor
            .pending_multicursor_positions
            .as_mut()
        {
            pending.retain(|key, _| {
                !known_ids.contains(&key.ns_id)
                    || live_marks
                        .get(&key.ns_id)
                        .map(|mark_ids| mark_ids.contains(&key.mark_id))
                        .unwrap_or(true)
            });
        }
    }
}

fn parse_multicursor_namespaces(value: &rmpv::Value) -> Result<HashMap<String, u64>, String> {
    let entries = value
        .as_map()
        .ok_or_else(|| "nvim_get_namespaces returned a non-map value".to_owned())?;
    Ok(entries
        .iter()
        .filter_map(|(name, id)| {
            let name = name.as_str()?.to_owned();
            if name == "nvim.multicursor" || name.starts_with("nvim.multicursor.") {
                Some((name, id.as_u64()?))
            } else {
                None
            }
        })
        .collect())
}

fn parse_extmark_ids(value: &rmpv::Value) -> Result<HashSet<u64>, String> {
    let marks = value
        .as_array()
        .ok_or_else(|| "nvim_buf_get_extmarks returned a non-array value".to_owned())?;
    Ok(marks
        .iter()
        .filter_map(|mark| mark.as_array()?.first()?.as_u64())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multicursor_namespace_query_filters_to_builtin_namespaces() {
        let value = rmpv::Value::Map(vec![
            (rmpv::Value::from("nvim.multicursor"), rmpv::Value::from(3)),
            (
                rmpv::Value::from("nvim.multicursor.cursor"),
                rmpv::Value::from(4),
            ),
            (rmpv::Value::from("plugin.marks"), rmpv::Value::from(9)),
        ]);

        assert_eq!(
            parse_multicursor_namespaces(&value).unwrap(),
            HashMap::from([
                ("nvim.multicursor".to_owned(), 3),
                ("nvim.multicursor.cursor".to_owned(), 4),
            ])
        );
    }

    #[test]
    fn multicursor_extmark_query_extracts_mark_ids() {
        let value = rmpv::Value::Array(vec![
            rmpv::Value::Array(vec![
                rmpv::Value::from(11),
                rmpv::Value::from(2),
                rmpv::Value::from(4),
            ]),
            rmpv::Value::Array(vec![
                rmpv::Value::from(12),
                rmpv::Value::from(6),
                rmpv::Value::from(8),
            ]),
        ]);

        assert_eq!(parse_extmark_ids(&value).unwrap(), HashSet::from([11, 12]));
    }
}

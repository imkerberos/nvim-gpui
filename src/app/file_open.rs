use super::NvimGpui;
use crate::nvim::NvimProcess;
use gpui::{Context, Window};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

const REMOTE_FILE_DROP_NOTICE: &str =
    "Remote mode does not support opening local files or directories.";
const FILE_DROP_NOTICE_DURATION: Duration = Duration::from_secs(4);

impl NvimGpui {
    pub(super) fn start_open_urls_task(
        &mut self,
        open_urls: async_channel::Receiver<Vec<String>>,
        cx: &mut Context<Self>,
    ) {
        self.window.open_urls_task = Some(cx.spawn(async move |weak, cx| {
            while let Ok(urls) = open_urls.recv().await {
                let paths = urls
                    .iter()
                    .filter_map(|url| path_from_open_url(url))
                    .collect::<Vec<_>>();
                if paths.is_empty() {
                    continue;
                }

                let requests = match weak.update(cx, |this, _cx| this.queue_open_files(paths)) {
                    Ok(requests) => requests,
                    Err(_) => break,
                };
                for request in requests {
                    match request.recv().await {
                        Ok(Ok(_)) => {}
                        Ok(Err(error)) => {
                            log::error!(
                                target: "nvim_gpui::startup",
                                "Neovim could not open a file from the platform: {error}"
                            );
                        }
                        Err(error) => {
                            log::warn!(
                                target: "nvim_gpui::startup",
                                "file-open request response was lost: {error}"
                            );
                        }
                    }
                }
            }
        }));
    }

    pub(super) fn on_file_drag_move(
        &mut self,
        _event: &gpui::DragMoveEvent<gpui::ExternalPaths>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .app
            .session
            .nvim
            .as_ref()
            .is_some_and(NvimProcess::is_remote)
            && self.window.file_drop_notice.is_none()
        {
            self.show_remote_file_drop_notice(cx);
        }
    }

    pub(super) fn on_file_drop(
        &mut self,
        paths: &gpui::ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.prevent_default();

        if self
            .app
            .session
            .nvim
            .as_ref()
            .is_some_and(NvimProcess::is_remote)
        {
            log::info!(
                target: "nvim_gpui::startup",
                "ignoring {} dropped path(s) in remote mode",
                paths.paths().len()
            );
            self.show_remote_file_drop_notice(cx);
            return;
        }

        let requests = self.queue_open_files(paths.paths().to_vec());
        if requests.is_empty() {
            return;
        }

        drop(cx.spawn(async move |_weak, _cx| {
            for request in requests {
                match request.recv().await {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        log::error!(
                            target: "nvim_gpui::startup",
                            "Neovim could not open a dropped path: {error}"
                        );
                    }
                    Err(error) => {
                        log::warn!(
                            target: "nvim_gpui::startup",
                            "dropped-path open request response was lost: {error}"
                        );
                    }
                }
            }
        }));
    }

    fn show_remote_file_drop_notice(&mut self, cx: &mut Context<Self>) {
        self.window.file_drop_notice_generation =
            self.window.file_drop_notice_generation.wrapping_add(1);
        let generation = self.window.file_drop_notice_generation;
        self.window.file_drop_notice = Some(REMOTE_FILE_DROP_NOTICE.to_owned());
        self.window.file_drop_notice_task = Some(cx.spawn(async move |weak, cx| {
            gpui::Timer::after(FILE_DROP_NOTICE_DURATION).await;
            let _ = weak.update(cx, |this, cx| {
                if this.window.file_drop_notice_generation == generation {
                    this.window.file_drop_notice = None;
                    this.window.file_drop_notice_task = None;
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn queue_open_files(
        &mut self,
        paths: Vec<PathBuf>,
    ) -> Vec<async_channel::Receiver<Result<rmpv::Value, String>>> {
        let Some(nvim) = self.app.session.nvim.as_ref() else {
            log::warn!(
                target: "nvim_gpui::startup",
                "ignoring platform file-open event because Neovim is unavailable"
            );
            return Vec::new();
        };

        paths
            .into_iter()
            .filter_map(|path| {
                log::info!(
                    target: "nvim_gpui::startup",
                    "opening platform file {}",
                    path.display()
                );
                match nvim.edit_file(&path) {
                    Ok(request) => Some(request),
                    Err(error) => {
                        log::error!(
                            target: "nvim_gpui::startup",
                            "could not queue platform file {}: {error}",
                            path.display()
                        );
                        None
                    }
                }
            })
            .collect()
    }
}

fn path_from_open_url(raw_url: &str) -> Option<PathBuf> {
    if raw_url.is_empty() {
        return None;
    }
    if Path::new(raw_url).is_absolute() {
        return Some(PathBuf::from(raw_url));
    }

    let Ok(url) = url::Url::parse(raw_url) else {
        return Some(PathBuf::from(raw_url));
    };
    if url.scheme() != "file" {
        log::warn!(
            target: "nvim_gpui::startup",
            "ignoring unsupported platform open URL scheme: {}",
            url.scheme()
        );
        return None;
    }

    match url.to_file_path() {
        Ok(path) => Some(path),
        Err(()) => {
            log::warn!(
                target: "nvim_gpui::startup",
                "could not convert platform file URL to a path: {raw_url}"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn file_open_urls_are_decoded_to_paths() {
        assert_eq!(
            path_from_open_url("file:///tmp/project%20notes.md"),
            Some(PathBuf::from("/tmp/project notes.md"))
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn file_open_urls_are_decoded_to_paths() {
        assert_eq!(
            path_from_open_url("file:///C:/project%20notes.md"),
            Some(PathBuf::from(r"C:\project notes.md"))
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn plain_file_open_paths_are_preserved() {
        assert_eq!(
            path_from_open_url("/tmp/project-notes.md"),
            Some(PathBuf::from("/tmp/project-notes.md"))
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn plain_file_open_paths_are_preserved() {
        assert_eq!(
            path_from_open_url(r"C:\project-notes.md"),
            Some(PathBuf::from(r"C:\project-notes.md"))
        );
    }

    #[test]
    fn non_file_open_urls_are_ignored() {
        assert_eq!(path_from_open_url("https://example.com/file.md"), None);
    }
}

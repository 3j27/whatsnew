use gpui::{
    App, Bounds, Context, SharedString, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use serde_json::Value;
use std::{
    env,
    process::{Command, Output},
};

const OMARCHY_REPO: &str = "https://github.com/omacom/omarchy";
const BG: u32 = 0x17191d;
const PANEL: u32 = 0x22252b;
const TEXT: u32 = 0xe8e9ec;
const MUTED: u32 = 0xa3a7b0;
const ACCENT: u32 = 0xf0c674;
const GREEN: u32 = 0x9ece6a;
const RED: u32 = 0xf7768e;

#[derive(Clone)]
struct PackageUpdate {
    name: String,
    old: String,
    new: String,
    upstream: Option<String>,
}

#[derive(Clone)]
struct CommitInfo {
    sha: String,
    title: String,
    author: String,
    diff: Option<String>,
    loading_diff: bool,
}

struct Review {
    omarchy_path: String,
    omarchy_version: String,
    package_name: String,
    installed_package_version: String,
    available_package_version: Option<String>,
    current_commit: Option<String>,
    target_commit: Option<String>,
    commits: Vec<CommitInfo>,
    compare_url: Option<String>,
    packages: Vec<PackageUpdate>,
    notice: String,
    busy: bool,
}

impl Review {
    fn installed_snapshot() -> Self {
        let omarchy_path =
            env::var("OMARCHY_PATH").unwrap_or_else(|_| "/usr/share/omarchy".to_owned());
        let omarchy_version = std::fs::read_to_string(format!("{omarchy_path}/version"))
            .map(|s| s.trim().to_owned())
            .unwrap_or_else(|_| "Unavailable".to_owned());
        let (package_name, installed_package_version) = installed_omarchy_package();
        let mut review = Self::loading();
        review.omarchy_path = omarchy_path;
        review.omarchy_version = omarchy_version;
        review.package_name = package_name;
        review.installed_package_version = installed_package_version;
        review
    }

    fn loading() -> Self {
        Self {
            omarchy_path: env::var("OMARCHY_PATH")
                .unwrap_or_else(|_| "/usr/share/omarchy".to_owned()),
            omarchy_version: "Checking…".to_owned(),
            package_name: "omarchy-dev".to_owned(),
            installed_package_version: "Checking…".to_owned(),
            available_package_version: None,
            current_commit: None,
            target_commit: None,
            commits: Vec::new(),
            compare_url: None,
            packages: Vec::new(),
            notice: "Checking installed and available package versions…".to_owned(),
            busy: false,
        }
    }

    fn load() -> Self {
        let omarchy_path =
            env::var("OMARCHY_PATH").unwrap_or_else(|_| "/usr/share/omarchy".to_owned());
        let omarchy_version = std::fs::read_to_string(format!("{omarchy_path}/version"))
            .map(|s| s.trim().to_owned())
            .unwrap_or_else(|_| "Unavailable".to_owned());

        let (package_name, installed_package_version) = installed_omarchy_package();
        let updates_output = run("checkupdates", &["--nocolor"]);
        let update_lines = updates_output
            .as_ref()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let packages = parse_updates(&update_lines);

        let available_package_version = packages
            .iter()
            .find(|p| p.name == package_name)
            .map(|p| p.new.clone());
        let current_commit = commit_from_package_version(&installed_package_version);
        let target_commit = available_package_version
            .as_deref()
            .and_then(commit_from_package_version);
        let compare_url = match (&current_commit, &target_commit) {
            (Some(old), Some(new)) => Some(format!("{OMARCHY_REPO}/compare/{old}...{new}")),
            _ => None,
        };
        let (commits, compare_error) = match (&current_commit, &target_commit) {
            (Some(old), Some(new)) => fetch_commits(old, new),
            _ => (Vec::new(), None),
        };

        let mut enriched = packages;
        for package in &mut enriched {
            package.upstream = package_upstream(&package.name);
        }

        let mut notice = String::new();
        if updates_output.is_err() {
            notice.push_str("Could not run checkupdates. Install pacman-contrib or inspect the error in a terminal. ");
        }
        if let Some(error) = compare_error {
            notice.push_str(&format!(
                "Could not fetch Omarchy commit details: {error}. "
            ));
        }
        if enriched.is_empty() && updates_output.is_ok() {
            notice.push_str("No package updates were reported by checkupdates. ");
        }
        if target_commit.is_none() && package_name == "omarchy-dev" {
            notice.push_str("No newer omarchy-dev package was found, so there is no target Omarchy commit to compare. ");
        }

        Self {
            omarchy_path,
            omarchy_version,
            package_name,
            installed_package_version,
            available_package_version,
            current_commit,
            target_commit,
            commits,
            compare_url,
            packages: enriched,
            notice,
            busy: false,
        }
    }
}

struct WhatsNew {
    review: Review,
    action_message: String,
    action_is_error: bool,
}

impl WhatsNew {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            review: Review::installed_snapshot(),
            action_message: String::new(),
            action_is_error: false,
        };
        view.refresh(cx);
        view
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.review.busy {
            return;
        }
        self.review.busy = true;
        self.action_message = "Refreshing package databases and update details…".to_owned();
        self.action_is_error = false;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let review = cx
                .background_executor()
                .spawn(async move { Review::load() })
                .await;
            this.update(cx, |this, cx| {
                this.review = review;
                this.action_message = "Review refreshed.".to_owned();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn toggle_commit_diff(&mut self, sha: String, cx: &mut Context<Self>) {
        let Some(commit) = self
            .review
            .commits
            .iter_mut()
            .find(|commit| commit.sha == sha)
        else {
            return;
        };
        if commit.diff.is_some() || commit.loading_diff {
            commit.diff = None;
            commit.loading_diff = false;
            cx.notify();
            return;
        }
        commit.loading_diff = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let fetch_sha = sha.clone();
            let diff = cx
                .background_executor()
                .spawn(async move { fetch_commit_diff(&fetch_sha) })
                .await;
            this.update(cx, |this, cx| {
                if let Some(commit) = this
                    .review
                    .commits
                    .iter_mut()
                    .find(|commit| commit.sha == sha)
                {
                    commit.diff = Some(diff);
                    commit.loading_diff = false;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn launch_update(&mut self, cx: &mut Context<Self>) {
        match Command::new("omarchy-launch-floating-terminal-with-presentation")
            .arg("omarchy-update")
            .spawn()
        {
            Ok(_) => {
                self.action_message =
                    "Omarchy update opened in a terminal. Review its confirmation before proceeding."
                        .to_owned();
                self.action_is_error = false;
            }
            Err(error) => {
                self.action_message = format!("Could not launch Omarchy update: {error}");
                self.action_is_error = true;
            }
        }
        cx.notify();
    }
}

impl Render for WhatsNew {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let review = &self.review;
        let mut content = div()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .w_full();

        content = content.child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child("What’s new"),
                        )
                        .child(div().text_sm().text_color(rgb(MUTED)).child(
                            "Review Omarchy and system package updates before applying them",
                        )),
                )
                .child(
                    div()
                        .id("refresh")
                        .px_4()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(PANEL))
                        .cursor_pointer()
                        .child(if review.busy {
                            "Refreshing…"
                        } else {
                            "Refresh"
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                ),
        );

        let installed_short = review
            .current_commit
            .as_deref()
            .unwrap_or("commit unavailable");
        let target_short = review
            .target_commit
            .as_deref()
            .unwrap_or("no package update");
        let omarchy_status = if review.available_package_version.is_some() {
            format!(
                "{} → {}",
                review.installed_package_version,
                review
                    .available_package_version
                    .as_deref()
                    .unwrap_or("unknown")
            )
        } else {
            format!(
                "{} (no newer package reported)",
                review.installed_package_version
            )
        };

        let mut omarchy_card = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .rounded_lg()
            .bg(rgb(PANEL))
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("Omarchy"),
            )
            .child(line(
                "Installed version file",
                review.omarchy_version.clone(),
            ))
            .child(line("Runtime source", review.omarchy_path.clone()))
            .child(line(
                format!("{} package", review.package_name),
                omarchy_status,
            ))
            .child(line("Source commit", installed_short.to_owned()))
            .child(line("Target commit", target_short.to_owned()));

        if let Some(url) = review.compare_url.clone() {
            let link = url.clone();
            omarchy_card = omarchy_card.child(
                div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .child(link_button("Open full GitHub diff", url, cx))
                    .child(div().text_sm().text_color(rgb(MUTED)).child(format!(
                        "{} commits in this source range",
                        review.commits.len()
                    ))),
            );
            let _ = link;
        } else {
            omarchy_card = omarchy_card.child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("A package-backed install has no Git checkout under /usr/share/omarchy. The package build version provides the source commit IDs instead."),
            );
        }

        if !review.commits.is_empty() {
            let mut commits = div().flex().flex_col().gap_1().pt_2();
            for commit in &review.commits {
                let sha = commit.sha.clone();
                let short_sha = commit.sha.chars().take(7).collect::<String>();
                let mut row = div().flex().flex_col().gap_1().py_1().child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(linked_commit_title(&commit.title, &commit.sha, cx))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child(commit.author.clone()),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("diff-{short_sha}")))
                                .text_xs()
                                .text_color(rgb(ACCENT))
                                .cursor_pointer()
                                .child(if commit.loading_diff {
                                    SharedString::from("Loading…")
                                } else {
                                    SharedString::from(short_sha.clone())
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_commit_diff(sha.clone(), cx)
                                })),
                        ),
                );
                if let Some(diff) = &commit.diff {
                    row = row.child(
                        div()
                            .p_2()
                            .rounded_md()
                            .bg(rgb(BG))
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(diff.clone()),
                    );
                }
                commits = commits.child(row);
            }
            omarchy_card = omarchy_card.child(commits);
        }
        let mut updates_columns = div()
            .flex()
            .flex_row()
            .items_start()
            .gap_4()
            .child(div().flex_1().child(omarchy_card));

        let mut package_card = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .rounded_lg()
            .bg(rgb(PANEL))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("System packages"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(format!("{} available", review.packages.len())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(div().flex_1().child("PACKAGE"))
                    .child(div().flex_1().child("INSTALLED → AVAILABLE"))
                    .child(div().child("UPSTREAM")),
            );

        for package in &review.packages {
            let mut row = div()
                .flex()
                .flex_row()
                .gap_2()
                .items_center()
                .border_t_1()
                .border_color(rgb(0x373b44))
                .pt_2()
                .child(div().flex_1().text_sm().child(package.name.clone()))
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .text_color(rgb(ACCENT))
                        .child(format!("{}  →  {}", package.old, package.new)),
                );
            if let Some(url) = package.upstream.clone() {
                row = row.child(link_button("Release notes / upstream", url, cx));
            } else {
                row = row.child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child("No upstream URL in pacman metadata"),
                );
            }
            package_card = package_card.child(row);
        }
        package_card = package_card.child(
            div()
                .pt_2()
                .text_xs()
                .text_color(rgb(MUTED))
                .child("Pacman packages do not provide a standard changelog field. Upstream links are best-effort; the Omarchy commit comparison is exact."),
        );
        updates_columns = updates_columns.child(div().flex_1().child(package_card));
        content = content.child(updates_columns);

        if !review.notice.is_empty() {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(rgb(ACCENT))
                    .child(review.notice.clone()),
            );
        }
        if !self.action_message.is_empty() {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(rgb(if self.action_is_error { RED } else { GREEN }))
                    .child(self.action_message.clone()),
            );
        }

        content = content.child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .items_center()
                .pt_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child("Preview only: this app does not install packages during refresh."),
                )
                .child(
                    div()
                        .id("update")
                        .px_5()
                        .py_3()
                        .rounded_md()
                        .bg(rgb(ACCENT))
                        .text_color(rgb(BG))
                        .font_weight(gpui::FontWeight::BOLD)
                        .cursor_pointer()
                        .child("Open Omarchy Update")
                        .on_click(cx.listener(|this, _, _, cx| this.launch_update(cx))),
                ),
        );

        content.id("review-scroll").size_full().overflow_y_scroll()
    }
}

fn line(label: impl Into<SharedString>, value: impl Into<SharedString>) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .gap_3()
        .text_sm()
        .child(div().w(px(190.)).text_color(rgb(MUTED)).child(label.into()))
        .child(div().child(value.into()))
}

fn link_button(
    label: impl Into<SharedString>,
    url: String,
    cx: &mut Context<WhatsNew>,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("link-{url}")))
        .px_2()
        .py_1()
        .rounded_md()
        .text_xs()
        .text_color(rgb(ACCENT))
        .cursor_pointer()
        .child(label.into())
        .on_click(cx.listener(move |_, _, _, _| open_url(&url)))
}

fn inline_link(label: String, url: String, cx: &mut Context<WhatsNew>) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("inline-link-{url}-{label}")))
        .text_sm()
        .text_color(rgb(ACCENT))
        .cursor_pointer()
        .child(label)
        .on_click(cx.listener(move |_, _, _, _| open_url(&url)))
}

fn linked_commit_title(title: &str, sha: &str, cx: &mut Context<WhatsNew>) -> impl IntoElement {
    let mut content = div().flex().flex_row().text_sm();
    let mut cursor = 0;
    while let Some(relative) = title[cursor..].find('#') {
        let start = cursor + relative;
        let end = title[start + 1..]
            .find(|c: char| !c.is_ascii_digit())
            .map_or(title.len(), |n| start + 1 + n);
        if end == start + 1 || (start > 0 && title.as_bytes()[start - 1].is_ascii_alphanumeric()) {
            content = content.child(inline_link(
                title[cursor..start + 1].to_owned(),
                format!("{OMARCHY_REPO}/commit/{sha}"),
                cx,
            ));
            cursor = start + 1;
            continue;
        }
        if start > cursor {
            content = content.child(inline_link(
                title[cursor..start].to_owned(),
                format!("{OMARCHY_REPO}/commit/{sha}"),
                cx,
            ));
        }
        content = content.child(inline_link(
            title[start..end].to_owned(),
            format!("{OMARCHY_REPO}/issues/{}", &title[start + 1..end]),
            cx,
        ));
        cursor = end;
    }
    if cursor < title.len() {
        content = content.child(inline_link(
            title[cursor..].to_owned(),
            format!("{OMARCHY_REPO}/commit/{sha}"),
            cx,
        ));
    }
    content
}

fn run(program: &str, args: &[&str]) -> Result<Output, String> {
    Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("{program}: {error}"))
}

fn installed_omarchy_package() -> (String, String) {
    for name in ["omarchy-dev", "omarchy"] {
        if let Ok(output) = run("pacman", &["-Q", name]) {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                let mut fields = text.split_whitespace();
                let package = fields.next().unwrap_or(name).to_owned();
                let version = fields.next().unwrap_or("unknown").to_owned();
                return (package, version);
            }
        }
    }
    (
        "omarchy".to_owned(),
        "not installed as a package".to_owned(),
    )
}

fn parse_updates(text: &str) -> Vec<PackageUpdate> {
    text.lines()
        .filter_map(|line| {
            let (before, new) = line.split_once(" -> ")?;
            let mut left = before.split_whitespace();
            Some(PackageUpdate {
                name: left.next()?.to_owned(),
                old: left.next()?.to_owned(),
                new: new.trim().to_owned(),
                upstream: None,
            })
        })
        .collect()
}

fn commit_from_package_version(version: &str) -> Option<String> {
    let pkgver = version
        .split_once('-')
        .map_or(version, |(pkgver, _)| pkgver);
    pkgver.split('.').find_map(|part| {
        let hash = part.strip_prefix('g')?;
        if hash.len() >= 7 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            Some(hash.to_owned())
        } else {
            None
        }
    })
}

fn package_upstream(name: &str) -> Option<String> {
    let output = run("pacman", &["-Si", name]).ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == "URL" && value.trim().starts_with("http"))
                .then(|| value.trim().to_owned())
        })
}

fn fetch_commits(old: &str, new: &str) -> (Vec<CommitInfo>, Option<String>) {
    let endpoint = format!("https://api.github.com/repos/omacom/omarchy/compare/{old}...{new}");
    let output = match Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--max-time",
            "12",
            "-H",
            "User-Agent: whatsnew",
            &endpoint,
        ])
        .output()
    {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            return (
                Vec::new(),
                Some(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
            );
        }
        Err(error) => return (Vec::new(), Some(format!("curl: {error}"))),
    };
    let json: Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(error) => {
            return (
                Vec::new(),
                Some(format!("invalid GitHub response: {error}")),
            );
        }
    };
    if let Some(message) = json.get("message").and_then(Value::as_str) {
        return (Vec::new(), Some(message.to_owned()));
    }
    let commits = json
        .get("commits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|commit| {
            let message = commit.pointer("/commit/message")?.as_str()?;
            let sha = commit.get("sha")?.as_str()?.to_owned();
            let author = commit
                .pointer("/author/login")
                .and_then(Value::as_str)
                .or_else(|| {
                    commit
                        .pointer("/commit/author/name")
                        .and_then(Value::as_str)
                })
                .unwrap_or("Unknown author")
                .to_owned();
            Some(CommitInfo {
                sha,
                title: message.lines().next().unwrap_or(message).to_owned(),
                author,
                diff: None,
                loading_diff: false,
            })
        })
        .collect();
    (commits, None)
}

fn fetch_commit_diff(sha: &str) -> String {
    let endpoint = format!("https://api.github.com/repos/omacom/omarchy/commits/{sha}");
    let output = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--max-time",
            "20",
            "-H",
            "User-Agent: whatsnew",
            "-H",
            "Accept: application/vnd.github+json",
            &endpoint,
        ])
        .output();
    let Ok(output) = output else {
        return "Could not fetch commit diff.".to_owned();
    };
    if !output.status.success() {
        return String::from_utf8_lossy(&output.stderr).trim().to_owned();
    }
    let Ok(json) = serde_json::from_slice::<Value>(&output.stdout) else {
        return "Invalid GitHub response.".to_owned();
    };
    let Some(files) = json.get("files").and_then(Value::as_array) else {
        return "No diff available.".to_owned();
    };
    let mut result = String::new();
    for file in files {
        if let Some(name) = file.get("filename").and_then(Value::as_str) {
            result.push_str(&format!("--- {name} ---\n"));
        }
        if let Some(patch) = file.get("patch").and_then(Value::as_str) {
            result.push_str(patch);
            result.push('\n');
        }
    }
    if result.is_empty() {
        "GitHub did not provide a text patch for this commit.".to_owned()
    } else {
        result
    }
}

fn open_url(url: &str) {
    let _ = Command::new("xdg-open").arg(url).spawn();
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1080.), px(820.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_window, cx| cx.new(WhatsNew::new),
        )
        .expect("open What’s New window");
    });
}

#[cfg(test)]
mod tests {
    use super::{commit_from_package_version, parse_updates};

    #[test]
    fn parses_checkupdates_rows_and_skips_non_rows() {
        let updates = parse_updates(
            "omarchy-dev 4.0.0.r7576.g5eb922f-1 -> 4.0.0.r7595.g50bb5db-1\ninvalid line\nlinux 6.1-1 -> 6.2-1\n",
        );

        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].name, "omarchy-dev");
        assert_eq!(updates[0].old, "4.0.0.r7576.g5eb922f-1");
        assert_eq!(updates[0].new, "4.0.0.r7595.g50bb5db-1");
        assert_eq!(updates[1].name, "linux");
    }

    #[test]
    fn extracts_embedded_source_commit_from_package_version() {
        assert_eq!(
            commit_from_package_version("4.0.0.r7576.g5eb922f-1"),
            Some("5eb922f".to_owned())
        );
        assert_eq!(commit_from_package_version("4.0.0-1"), None);
        assert_eq!(commit_from_package_version("r12.gabc-1"), None);
    }
}

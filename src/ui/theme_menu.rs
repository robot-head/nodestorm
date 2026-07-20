//! Theme picker dropdown: color-mode switch plus the palette-family list,
//! with live swatches (each row locally re-scopes `data-theme` so the
//! swatch dots resolve that family's tokens in the current mode).

use dioxus::prelude::*;

use crate::cli::Cli;
use crate::prefs::Preferences;
use crate::theme::{self, Mode};

/// Persist the preference; the in-memory signal already changed, so a
/// failed write only warns and the UI stays responsive.
fn persist(cli: &Cli, prefs: &Preferences) {
    let outcome = cli
        .prefs_path()
        .and_then(|path| crate::prefs::save(&path, prefs));
    if let Err(err) = outcome {
        tracing::warn!(%err, "saving preferences failed");
    }
}

/// The theme dropdown body: mode row + family list. Lives inside the ⋯
/// menu's accordion. Mode clicks keep the menu open; family clicks close
/// it via `on_pick`.
#[component]
pub fn ThemeRows(on_pick: EventHandler<()>) -> Element {
    let cli = use_context::<Cli>();
    let mut prefs = use_context::<super::ThemePref>().0;
    let active_theme = prefs.read().theme.clone();
    let active_mode = prefs.read().mode;

    rsx! {
        div { class: "mode-row",
            for (mode, label) in [
                (Mode::Auto, "Auto"),
                (Mode::Light, "Light"),
                (Mode::Dark, "Dark"),
            ] {
                button {
                    key: "{label}",
                    class: if active_mode == mode { "mode-btn active" } else { "mode-btn" },
                    title: "Auto follows the system setting",
                    onclick: {
                        let cli = cli.clone();
                        move |_| {
                            prefs.write().mode = mode;
                            persist(&cli, &prefs.read());
                        }
                    },
                    "{label}"
                }
            }
        }
        for f in theme::FAMILIES {
            button {
                key: "{f.id}",
                class: if active_theme == f.id { "theme-row active" } else { "theme-row" },
                onclick: {
                    let cli = cli.clone();
                    move |_| {
                        prefs.write().theme = f.id.to_owned();
                        persist(&cli, &prefs.read());
                        on_pick.call(());
                    }
                },
                span { class: "theme-name", "{f.name}" }
                span {
                    class: "theme-swatches",
                    "data-theme": "{f.id}",
                    span { class: "swatch swatch-bg" }
                    span { class: "swatch swatch-accent" }
                    span { class: "swatch swatch-modified" }
                    span { class: "swatch swatch-affected" }
                }
                if active_theme == f.id {
                    span { class: "theme-check", "✓" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persist_writes_the_selected_preferences() {
        let path =
            std::env::temp_dir().join(format!("nodestorm-theme-prefs-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let cli = Cli {
            port: 4747,
            session: None,
            sessions_dir: None,
            prefs: Some(path.clone()),
            demo: false,
            demo_big: None,
            headless: true,
            window_size: None,
        };
        let prefs = Preferences {
            version: Preferences::VERSION,
            theme: "solarized".into(),
            mode: Mode::Dark,
            recent_repositories: vec![],
        };

        persist(&cli, &prefs);

        assert2::assert!((crate::prefs::load_or_default(&path)) == (prefs));
        std::fs::remove_file(path).unwrap();
    }
}

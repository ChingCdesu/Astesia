use std::sync::Arc;

use editor::Editor;
use gpui::{App, AppContext as _, Context, Global, Subscription, Window};
use language::{BracketPair, BracketPairConfig, Buffer, Language, LanguageConfig, LanguageMatcher};
use theme::ActiveTheme as _;

struct SqlLanguage {
    language: Arc<Language>,
    _theme_subscription: Subscription,
}

impl Global for SqlLanguage {}

pub(super) fn init(cx: &mut App) {
    let language = build_language().expect("failed to initialize bundled SQL language");
    language.set_theme(cx.theme().syntax());
    let theme_subscription = cx.observe_global::<theme::GlobalTheme>({
        let language = language.clone();
        move |cx| language.set_theme(cx.theme().syntax())
    });
    cx.set_global(SqlLanguage {
        language,
        _theme_subscription: theme_subscription,
    });
}

pub(super) fn editor(text: &str, window: &mut Window, cx: &mut Context<Editor>) -> Editor {
    let language = cx.global::<SqlLanguage>().language.clone();
    let buffer = cx.new(|cx| Buffer::local(text, cx).with_language(language, cx));
    Editor::for_buffer(buffer, None, window, cx)
}

fn build_language() -> anyhow::Result<Arc<Language>> {
    let language = Language::new(
        LanguageConfig {
            name: "SQL".into(),
            grammar: Some("sql".into()),
            matcher: LanguageMatcher {
                path_suffixes: vec!["sql".to_string()],
                ..Default::default()
            }
            .into(),
            line_comments: vec!["-- ".into()],
            brackets: BracketPairConfig {
                pairs: vec![
                    bracket("(", ")", false),
                    bracket("'", "'", false),
                    bracket("\"", "\"", false),
                    bracket("BEGIN", "END", true),
                ],
                ..Default::default()
            },
            ..Default::default()
        },
        Some(tree_sitter_sequel::LANGUAGE.into()),
    )
    .with_highlights_query(include_str!("sql_language/highlights.scm"))?;
    Ok(Arc::new(language))
}

fn bracket(start: &str, end: &str, newline: bool) -> BracketPair {
    BracketPair {
        start: start.to_string(),
        end: end.to_string(),
        close: true,
        surround: true,
        newline,
    }
}

#[cfg(test)]
mod tests {
    use gpui::{rgba, HighlightStyle, TestAppContext};
    use language::{HighlightId, LanguageAwareStyling};
    use theme::SyntaxTheme;

    use super::*;

    #[gpui::test]
    fn bundled_sql_language_highlights_keywords_and_identifiers(cx: &mut TestAppContext) {
        let language = build_language().expect("SQL language");
        let syntax_theme = SyntaxTheme::new([
            (
                "keyword".to_string(),
                HighlightStyle::color(rgba(0xff0000ff).into()),
            ),
            (
                "variable".to_string(),
                HighlightStyle::color(rgba(0x00ff00ff).into()),
            ),
        ]);
        language.set_theme(&syntax_theme);
        let keyword = syntax_theme.highlight_id("keyword").map(HighlightId::new);
        let variable = syntax_theme.highlight_id("variable").map(HighlightId::new);
        let source = "SELECT customer_id FROM customers;";
        let buffer = cx.new(|cx| Buffer::local(source, cx).with_language(language, cx));

        cx.run_until_parked();
        buffer.read_with(cx, |buffer, _| {
            let snapshot = buffer.snapshot();
            assert_eq!(snapshot.language().unwrap().name().as_ref(), "SQL");
            let highlighted = snapshot
                .chunks(
                    0..snapshot.len(),
                    LanguageAwareStyling {
                        tree_sitter: true,
                        diagnostics: false,
                    },
                )
                .filter_map(|chunk| {
                    matches!(chunk.syntax_highlight_id, id if id == keyword || id == variable)
                        .then(|| chunk.text.to_string())
                })
                .collect::<Vec<_>>();

            assert!(highlighted.iter().any(|text| text == "SELECT"));
            assert!(highlighted.iter().any(|text| text == "customer_id"));
        });
    }
}

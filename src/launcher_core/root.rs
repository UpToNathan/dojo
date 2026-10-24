//! The launcher's root list: applications, commands and inline rows for one query.

use nucleo_matcher::{Matcher, Utf32String};

use crate::application_catalogue::{Catalogue, QueryPatterns, RESULT_LIMIT, Scored};
use crate::launch_history::{LaunchHistory, TermMatch};

use super::commands::{Command, HiddenCommands};
use super::rows::{Row, RowAction, RowIcon};

pub struct RootInput<'a> {
    pub catalogue: &'a Catalogue,
    pub query: &'a str,
    pub history: &'a LaunchHistory,
    pub now_ms: u64,
    /// The commands whose views exist.
    pub commands: &'a [Command],
    pub hidden: &'a HiddenCommands,
    /// Rows engines produced for this query (a calculation, a bang, ...). They lead the list.
    pub inline: Vec<Row>,
}

fn command_row(command: &Command) -> Row {
    Row {
        id: command.id.history_id(),
        title: command.title.to_owned(),
        subtitle: command.subtitle.to_owned(),
        icon: RowIcon::Glyph(command.glyph.to_owned()),
        action: RowAction::Open(command.id),
    }
}

fn application_row(catalogue: &Catalogue, index: usize) -> Row {
    let app = &catalogue.applications[index];
    Row {
        id: app.id.clone(),
        title: app.name.clone(),
        subtitle: app.description.clone(),
        icon: RowIcon::Application(index),
        action: RowAction::Launch(index),
    }
}

/// With no query: a Suggestions group of the commands the user has not hidden (most used first,
/// a fresh install's order otherwise), then the applications in use first. With a query: inline
/// rows, then applications and commands ranked together by the same tiers.
pub fn root_rows(input: RootInput<'_>, matcher: &mut Matcher) -> Vec<Row> {
    let RootInput {
        catalogue,
        query,
        history,
        now_ms,
        commands,
        hidden,
        inline,
    } = input;
    if query.is_empty() {
        let mut suggestions = commands
            .iter()
            .enumerate()
            .filter(|(_, command)| !hidden.contains(command.id))
            .map(|(position, command)| {
                (
                    position,
                    history.frecency(&command.id.history_id(), now_ms),
                    command,
                )
            })
            .collect::<Vec<_>>();
        suggestions.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        let apps = catalogue.search_ranked("", matcher, history, now_ms);
        let mut rows = Vec::new();
        if !suggestions.is_empty() {
            rows.push(Row::header("Suggestions"));
            rows.extend(
                suggestions
                    .into_iter()
                    .map(|(_, _, command)| command_row(command)),
            );
            if !apps.is_empty() {
                rows.push(Row::header("Applications"));
            }
        }
        rows.extend(
            apps.into_iter()
                .map(|index| application_row(catalogue, index)),
        );
        return rows;
    }

    enum Hit {
        Application(usize),
        Command(usize),
    }
    let patterns = QueryPatterns::new(query);
    let mut hits = catalogue
        .scored(query, matcher, history, now_ms)
        .into_iter()
        .map(|scored| (Hit::Application(scored.index), scored))
        .collect::<Vec<_>>();
    // Commands are ranked after the applications' positions, so ties keep applications first.
    let base = catalogue.applications.len();
    for (position, command) in commands.iter().enumerate() {
        let id = command.id.history_id();
        let name = Utf32String::from(command.title);
        let metadata = Utf32String::from(
            format!(
                "{} {} {}",
                command.title,
                command.subtitle,
                command.keywords.join(" ")
            )
            .as_str(),
        );
        let matched = patterns.score(&name, &metadata, matcher);
        let learned = match history.term_match(&id, query, now_ms) {
            TermMatch::Exact => Some(0),
            TermMatch::Prefix => Some(1),
            TermMatch::None => None,
        };
        let (tier, score) = match (matched, learned) {
            (Some((tier, score)), Some(learned)) => (tier.min(learned), score),
            (Some(found), None) => found,
            (None, Some(learned)) => (learned, 0),
            (None, None) => continue,
        };
        hits.push((
            Hit::Command(position),
            Scored {
                index: base + position,
                tier,
                score,
                frecency: history.frecency(&id, now_ms),
            },
        ));
    }
    hits.sort_by(|left, right| left.1.compare(&right.1));
    hits.truncate(RESULT_LIMIT);
    let mut rows = inline;
    rows.extend(hits.into_iter().map(|(hit, _)| match hit {
        Hit::Application(index) => application_row(catalogue, index),
        Hit::Command(position) => command_row(&commands[position]),
    }));
    rows
}

#[cfg(test)]
mod tests {
    use super::super::commands::{ALL, CommandId};
    use super::*;
    use crate::application_catalogue::Application;

    const NOW: u64 = 1_800_000_000_000;

    fn catalogue() -> Catalogue {
        Catalogue {
            applications: vec![
                Application::new("files.desktop", "Files", "File manager"),
                Application::new("firefox.desktop", "Firefox", "Web Browser"),
                Application::new("emacs.desktop", "Emacs", "Text editor"),
            ],
        }
    }

    fn commands() -> Vec<Command> {
        vec![ALL[0], ALL[1], ALL[2]]
    }

    fn rows(query: &str, history: &LaunchHistory, hidden: &HiddenCommands) -> Vec<Row> {
        let catalogue = catalogue();
        let commands = commands();
        root_rows(
            RootInput {
                catalogue: &catalogue,
                query,
                history,
                now_ms: NOW,
                commands: &commands,
                hidden,
                inline: Vec::new(),
            },
            &mut Matcher::new(nucleo_matcher::Config::DEFAULT),
        )
    }

    fn titles(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|row| row.title.as_str()).collect()
    }

    #[test]
    fn empty_query_lists_suggestions_then_applications() {
        let rows = rows("", &LaunchHistory::default(), &HiddenCommands::default());
        assert_eq!(
            titles(&rows),
            [
                "Suggestions",
                "Search Emoji",
                "Clipboard History",
                "Switch Windows",
                "Applications",
                "Files",
                "Firefox",
                "Emacs"
            ]
        );
        assert_eq!(rows[1].action, RowAction::Open(CommandId::Emoji));
        assert_eq!(rows[5].action, RowAction::Launch(0));
    }

    #[test]
    fn used_commands_rise_and_hidden_ones_leave_the_suggestions() {
        let mut history = LaunchHistory::default();
        history.record(&CommandId::Windows.history_id(), "", NOW);
        history.record("emacs.desktop", "", NOW);
        let mut hidden = HiddenCommands::default();
        hidden.toggle(CommandId::ClipboardHistory);
        let rows = rows("", &history, &hidden);
        assert_eq!(
            titles(&rows),
            [
                "Suggestions",
                "Switch Windows",
                "Search Emoji",
                "Applications",
                "Emacs",
                "Files",
                "Firefox"
            ]
        );
    }

    #[test]
    fn without_commands_the_empty_list_is_just_applications() {
        let catalogue = catalogue();
        let rows = root_rows(
            RootInput {
                catalogue: &catalogue,
                query: "",
                history: &LaunchHistory::default(),
                now_ms: NOW,
                commands: &[],
                hidden: &HiddenCommands::default(),
                inline: Vec::new(),
            },
            &mut Matcher::new(nucleo_matcher::Config::DEFAULT),
        );
        assert_eq!(titles(&rows), ["Files", "Firefox", "Emacs"]);
    }

    #[test]
    fn typed_commands_rank_with_applications_and_hidden_ones_still_match() {
        let mut hidden = HiddenCommands::default();
        hidden.toggle(CommandId::Emoji);
        let found = rows("emoji", &LaunchHistory::default(), &hidden);
        assert_eq!(
            found.first().map(|row| row.title.as_str()),
            Some("Search Emoji")
        );
        let found = rows("fi", &LaunchHistory::default(), &HiddenCommands::default());
        assert!(titles(&found).contains(&"Files") && titles(&found).contains(&"Firefox"));
        assert!(!found.iter().any(|row| row.action == RowAction::Header));
    }

    #[test]
    fn a_learned_term_can_open_a_command() {
        let mut history = LaunchHistory::default();
        history.record(&CommandId::ClipboardHistory.history_id(), "cb", NOW);
        let found = rows("cb", &history, &HiddenCommands::default());
        assert_eq!(
            found.first().map(|row| row.title.as_str()),
            Some("Clipboard History")
        );
    }

    #[test]
    fn inline_rows_lead_typed_results() {
        let catalogue = catalogue();
        let commands = commands();
        let inline = Row {
            id: String::new(),
            title: "= 42".into(),
            subtitle: "Calculator".into(),
            icon: RowIcon::Glyph("=".into()),
            action: RowAction::Copy("42".into()),
        };
        let rows = root_rows(
            RootInput {
                catalogue: &catalogue,
                query: "fi",
                history: &LaunchHistory::default(),
                now_ms: NOW,
                commands: &commands,
                hidden: &HiddenCommands::default(),
                inline: vec![inline.clone()],
            },
            &mut Matcher::new(nucleo_matcher::Config::DEFAULT),
        );
        assert_eq!(rows.first(), Some(&inline));
        assert!(rows.len() > 1);
    }

    #[test]
    fn frecency_orders_commands_within_a_tier() {
        let mut history = LaunchHistory::default();
        history.record(&CommandId::Windows.history_id(), "", NOW);
        // "w" is a prefix of Switch Windows only by metadata; both emoji and clipboard have no w.
        let found = rows("s", &history, &HiddenCommands::default());
        assert_eq!(
            found.first().map(|row| row.title.as_str()),
            Some("Switch Windows")
        );
    }
}

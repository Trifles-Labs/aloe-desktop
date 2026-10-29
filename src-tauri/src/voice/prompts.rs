//! Questions and approvals a turn stops for, turned into something said and answered out loud.
//!
//! A voice turn can still stop for the user: a sensitive-file "Allow / Deny" card, an email or
//! calendar action waiting for approval, a command waiting in "ask" mode. Each becomes a `Prompt`,
//! read out with exactly what will happen, then answered by voice. The matching is deliberately
//! strict where it acts: only a clear yes approves, and a negation anywhere wins.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Prompt {
    /// An `ask_question` card. Answered at `/api/questions/:id/answer`.
    Question { id: String, question: String, mode: QuestionMode, options: Vec<Choice> },
    /// An email, calendar or GitHub action waiting for approval (`/api/approvals/:id`).
    ConnectorApproval { id: String, summary: String },
    /// A command this computer is holding in "ask" mode.
    CommandApproval { job_id: String, command: String, reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionMode {
    Single,
    Multi,
    FreeText,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub label: String,
    /// What the answer sends: the option's value, else its label (as the chat page does).
    pub value: String,
}

/// What the user's answer came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// Send this as the answer.
    Text(String),
    Yes,
    No,
    /// Couldn't tell. Ask again (once).
    Unclear,
}

impl Prompt {
    /// A pending card from the chat stream that voice has to handle, if this widget is one.
    pub fn from_widget(widget: &Value) -> Option<Prompt> {
        let text = |key: &str| widget.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
        let pending = widget.get("status").and_then(Value::as_str).map_or(true, |s| s == "pending");
        match widget.get("kind").and_then(Value::as_str)? {
            "ask_question" if pending => {
                let id = text("id").strip_prefix("ask_question-")?.to_string();
                let mode = match text("mode").as_str() {
                    "single_choice" => QuestionMode::Single,
                    "multi_select" => QuestionMode::Multi,
                    _ => QuestionMode::FreeText,
                };
                let options = widget
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|options| {
                        options
                            .iter()
                            .filter_map(|option| {
                                let label = option.get("label").and_then(Value::as_str)?.trim().to_string();
                                let value = option.get("value").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| label.clone());
                                Some(Choice { label, value })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Some(Prompt::Question { id, question: text("question"), mode, options })
            }
            "approval_request" => {
                let action = widget.get("action")?;
                if action.get("status").and_then(Value::as_str).is_some_and(|s| s != "pending") {
                    return None;
                }
                let id = action.get("id").and_then(Value::as_str)?.to_string();
                Some(Prompt::ConnectorApproval { id, summary: describe_action(action) })
            }
            _ => None,
        }
    }

    /// Whether a yes/no answer decides it, rather than a choice or free text.
    fn is_yes_no(&self) -> bool {
        matches!(self, Prompt::ConnectorApproval { .. } | Prompt::CommandApproval { .. })
    }

    /// What Aloe says to ask.
    pub fn read_back(&self) -> String {
        match self {
            Prompt::Question { question, mode, options, .. } => {
                let mut said = ensure_stop(question);
                if !options.is_empty() && *mode != QuestionMode::FreeText {
                    let listed: Vec<String> = options.iter().enumerate().map(|(i, c)| format!("Option {}: {}.", NUMBER_WORDS[i + 1], c.label)).collect();
                    said.push(' ');
                    said.push_str(&listed.join(" "));
                    said.push_str(if *mode == QuestionMode::Multi { " Say the numbers or the names you want." } else { " Say the number or the name." });
                } else {
                    said.push_str(" Go ahead and answer.");
                }
                said
            }
            Prompt::ConnectorApproval { summary, .. } => format!("{} Should I go ahead? Say yes or no.", ensure_stop(summary)),
            Prompt::CommandApproval { command, reason, .. } => {
                let shown: String = command.chars().take(120).collect();
                let cut = if command.chars().count() > 120 { ", and more" } else { "" };
                let why = if reason.trim().is_empty() { String::new() } else { format!(" {}", ensure_stop(reason)) };
                format!("Aloe wants to run a command on this computer.{why} The command is: {shown}{cut}. Say yes to allow it, or no.")
            }
        }
    }

    /// What Aloe says when the first answer didn't settle it.
    pub fn ask_again(&self) -> &'static str {
        match self {
            Prompt::Question { mode: QuestionMode::FreeText, .. } => "Sorry, I didn't catch that. Please say your answer again.",
            Prompt::Question { .. } => "Sorry, I didn't catch that. Please say the number of your choice.",
            _ => "Sorry, I didn't catch that. Please say yes or no.",
        }
    }

    pub fn understand(&self, heard: &str) -> Answer {
        if heard.trim().is_empty() {
            return Answer::Unclear;
        }
        if self.is_yes_no() {
            return match_yes_no(heard);
        }
        let Prompt::Question { mode, options, .. } = self else { return Answer::Unclear };
        match mode {
            QuestionMode::FreeText => Answer::Text(heard.trim().to_string()),
            QuestionMode::Single => match match_choice(heard, options) {
                Some(i) => Answer::Text(options[i].value.clone()),
                // "Allow / Deny" style cards: a plain yes or no picks the matching option.
                None => match match_yes_no(heard) {
                    Answer::Yes => affirmative_option(options).map_or(Answer::Unclear, |c| Answer::Text(c.value.clone())),
                    Answer::No => Answer::No,
                    _ => Answer::Unclear,
                },
            },
            QuestionMode::Multi => {
                let picked = match_choices(heard, options);
                if picked.is_empty() {
                    if match_yes_no(heard) == Answer::No {
                        Answer::No
                    } else {
                        Answer::Unclear
                    }
                } else {
                    Answer::Text(picked.iter().map(|&i| options[i].value.clone()).collect::<Vec<_>>().join(", "))
                }
            }
        }
    }
}

/// "Send an email to priya@x.com: Running late." from a proposed action's preview.
fn describe_action(action: &Value) -> String {
    let preview = action.get("preview").cloned().unwrap_or(Value::Null);
    let field = |key: &str| preview.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let kind = action.get("actionType").and_then(Value::as_str).unwrap_or("action").replace('_', " ");
    if let Some(description) = action.get("description").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        return ensure_stop(description);
    }
    let mut said = format!("Aloe wants to {kind}");
    if let Some(to) = field("to") {
        said.push_str(&format!(" to {to}"));
    }
    if let Some(subject) = field("subject").or_else(|| field("summary")).or_else(|| field("title")) {
        said.push_str(&format!(", about {subject}"));
    }
    if let Some(start) = field("start").or_else(|| field("startTime")) {
        said.push_str(&format!(", starting {start}"));
    }
    said.push('.');
    said
}

fn ensure_stop(text: &str) -> String {
    let text = text.trim();
    if text.ends_with(['.', '?', '!']) {
        text.to_string()
    } else {
        format!("{text}.")
    }
}

const NUMBER_WORDS: [&str; 11] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];
const ORDINALS: [&str; 11] = ["", "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth"];

/// Lowercase words, punctuation gone.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .map(|w| w.trim_matches('\'').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

/// A number the user said as a digit, a word or an ordinal ("2", "two", "second", "number two").
fn spoken_number(word: &str) -> Option<usize> {
    if let Ok(n) = word.parse::<usize>() {
        return Some(n);
    }
    NUMBER_WORDS.iter().position(|w| *w == word).or_else(|| ORDINALS.iter().position(|w| !w.is_empty() && *w == word)).filter(|&n| n > 0)
}

/// Every option the answer names, by number or by its label's words, in option order.
fn match_choices(heard: &str, options: &[Choice]) -> Vec<usize> {
    let said = words(heard);
    let mut picked: Vec<usize> = said.iter().filter_map(|w| spoken_number(w)).filter(|&n| n >= 1 && n <= options.len()).map(|n| n - 1).collect();
    for (i, option) in options.iter().enumerate() {
        let label = words(&option.label);
        if !label.is_empty() && contains_run(&said, &label) {
            picked.push(i);
        }
    }
    picked.sort_unstable();
    picked.dedup();
    picked
}

/// Exactly one option, or nothing when the answer is ambiguous.
pub fn match_choice(heard: &str, options: &[Choice]) -> Option<usize> {
    match match_choices(heard, options).as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

fn contains_run(haystack: &[String], needle: &[String]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

fn affirmative_option(options: &[Choice]) -> Option<&Choice> {
    const YES_LABELS: [&str; 7] = ["allow", "yes", "approve", "continue", "confirm", "ok", "okay"];
    options.iter().find(|c| words(&c.label).first().is_some_and(|w| YES_LABELS.contains(&w.as_str())))
}

/// Strict yes/no. Any "no" word anywhere wins, so "yes, no wait" and "don't send it" are no; a
/// yes needs one of a short list of words and nothing that negates it.
pub fn match_yes_no(heard: &str) -> Answer {
    const NO: [&str; 13] = ["no", "nope", "don't", "dont", "do not", "stop", "cancel", "deny", "reject", "wait", "never", "not", "nah"];
    const YES: [&str; 14] = ["yes", "yeah", "yep", "yup", "sure", "approve", "approved", "allow", "ok", "okay", "go ahead", "do it", "send it", "confirm"];
    let said = words(heard);
    let joined = format!(" {} ", said.join(" "));
    let has = |phrase: &str| joined.contains(&format!(" {phrase} "));
    if NO.iter().any(|w| has(w)) {
        Answer::No
    } else if YES.iter().any(|w| has(w)) {
        Answer::Yes
    } else {
        Answer::Unclear
    }
}

/// Things said to voice itself rather than to Aloe. Only these exact short phrases count, so an
/// ordinary request that happens to start with "stop" or "repeat" still goes to Aloe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalCommand {
    Repeat,
    Quiet,
    Slower,
    Faster,
}

pub fn local_command(heard: &str) -> Option<LocalCommand> {
    let said = words(heard).join(" ");
    let said = said.trim_start_matches("aloe ").trim_start_matches("please ").trim_end_matches(" please");
    match said {
        "repeat" | "repeat that" | "say that again" | "say it again" | "what did you say" | "come again" | "again" => Some(LocalCommand::Repeat),
        "stop" | "stop talking" | "be quiet" | "quiet" | "shush" | "shut up" | "never mind" | "nevermind" => Some(LocalCommand::Quiet),
        "slower" | "speak slower" | "talk slower" | "slow down" | "speak more slowly" | "more slowly" => Some(LocalCommand::Slower),
        "faster" | "speak faster" | "talk faster" | "speed up" | "speak more quickly" | "more quickly" => Some(LocalCommand::Faster),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn options(labels: &[&str]) -> Vec<Choice> {
        labels.iter().map(|l| Choice { label: l.to_string(), value: l.to_lowercase() }).collect()
    }

    #[test]
    fn choices_by_number_ordinal_or_name() {
        let opts = options(&["Italian place", "Sushi bar", "Pizza"]);
        assert_eq!(match_choice("two", &opts), Some(1));
        assert_eq!(match_choice("Option 3.", &opts), Some(2));
        assert_eq!(match_choice("the first one please", &opts), Some(0));
        assert_eq!(match_choice("let's do the sushi bar", &opts), Some(1));
        assert_eq!(match_choice("pizza or sushi bar", &opts), None, "two names is ambiguous");
        assert_eq!(match_choice("seven", &opts), None, "out of range");
        assert_eq!(match_choice("whatever you think", &opts), None);
    }

    #[test]
    fn multi_select_collects_every_named_option() {
        let prompt = Prompt::Question { id: "q".into(), question: "Which?".into(), mode: QuestionMode::Multi, options: options(&["Email", "Calendar", "Drive"]) };
        assert_eq!(prompt.understand("one and three"), Answer::Text("email, drive".into()));
        assert_eq!(prompt.understand("calendar and email"), Answer::Text("email, calendar".into()));
        assert_eq!(prompt.understand("none of them, no"), Answer::No);
        assert_eq!(prompt.understand("hmm"), Answer::Unclear);
    }

    #[test]
    fn yes_needs_a_clear_yes_and_any_no_wins() {
        for yes in ["Yes.", "yeah go ahead", "Okay, send it", "approve", "sure"] {
            assert_eq!(match_yes_no(yes), Answer::Yes, "{yes}");
        }
        for no in ["No.", "yes, no wait", "don't send it", "do not do that", "stop", "cancel it", "not yet"] {
            assert_eq!(match_yes_no(no), Answer::No, "{no}");
        }
        for unclear in ["maybe", "what was that", "hmm", ""] {
            assert_eq!(match_yes_no(unclear), Answer::Unclear, "{unclear}");
        }
    }

    #[test]
    fn allow_deny_cards_take_yes_and_no() {
        let prompt = Prompt::Question {
            id: "q".into(),
            question: "Allow Aloe to read .env?".into(),
            mode: QuestionMode::Single,
            options: vec![Choice { label: "Allow".into(), value: "allow".into() }, Choice { label: "Deny".into(), value: "deny".into() }],
        };
        assert_eq!(prompt.understand("yes"), Answer::Text("allow".into()));
        assert_eq!(prompt.understand("deny"), Answer::Text("deny".into()));
        assert_eq!(prompt.understand("no"), Answer::No);
        assert_eq!(prompt.understand("uh"), Answer::Unclear);
    }

    #[test]
    fn approvals_only_take_yes_or_no() {
        let prompt = Prompt::CommandApproval { job_id: "j".into(), command: "dir".into(), reason: "List files".into() };
        assert_eq!(prompt.understand("yes"), Answer::Yes);
        assert_eq!(prompt.understand("option one"), Answer::Unclear);
        assert_eq!(prompt.understand(""), Answer::Unclear);
    }

    #[test]
    fn widgets_become_prompts_only_while_pending() {
        let question = json!({ "kind": "ask_question", "id": "ask_question-abc", "question": "Which one", "mode": "single_choice",
            "options": [{ "label": "A" }, { "label": "B", "value": "b-value" }], "status": "pending" });
        assert_eq!(
            Prompt::from_widget(&question),
            Some(Prompt::Question { id: "abc".into(), question: "Which one".into(), mode: QuestionMode::Single, options: vec![
                Choice { label: "A".into(), value: "A".into() },
                Choice { label: "B".into(), value: "b-value".into() },
            ] })
        );
        let answered = json!({ "kind": "ask_question", "id": "ask_question-abc", "question": "Which one", "mode": "free_text", "status": "answered" });
        assert_eq!(Prompt::from_widget(&answered), None);

        let email = json!({ "kind": "approval_request", "id": "w", "action": { "id": "act1", "actionType": "send_email", "status": "pending",
            "preview": { "to": "priya@example.com", "subject": "Running late" } } });
        let Some(Prompt::ConnectorApproval { id, summary }) = Prompt::from_widget(&email) else { panic!("not an approval") };
        assert_eq!(id, "act1");
        assert_eq!(summary, "Aloe wants to send email to priya@example.com, about Running late.");
        assert_eq!(Prompt::from_widget(&json!({ "kind": "weather" })), None);
    }

    #[test]
    fn read_back_says_what_will_happen() {
        let prompt = Prompt::Question { id: "q".into(), question: "Which restaurant".into(), mode: QuestionMode::Single, options: options(&["Italian", "Sushi"]) };
        assert_eq!(prompt.read_back(), "Which restaurant. Option one: Italian. Option two: Sushi. Say the number or the name.");
        let long = "x".repeat(200);
        let said = Prompt::CommandApproval { job_id: "j".into(), command: long, reason: "".into() }.read_back();
        assert!(said.contains("and more") && said.ends_with("Say yes to allow it, or no."));
    }

    #[test]
    fn local_commands_are_exact_phrases_only() {
        assert_eq!(local_command("Say that again."), Some(LocalCommand::Repeat));
        assert_eq!(local_command("Aloe, stop"), Some(LocalCommand::Quiet));
        assert_eq!(local_command("slower please"), Some(LocalCommand::Slower));
        assert_eq!(local_command("Speak faster"), Some(LocalCommand::Faster));
        assert_eq!(local_command("stop the music on Spotify"), None);
        assert_eq!(local_command("repeat my last email to Priya"), None);
    }
}

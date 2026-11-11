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

    /// Whether an answer `understand` called Unclear is worth a second opinion from the backend.
    /// Not for free text (nothing to interpret) or multi-select (several picks don't map to one).
    /// Never for anything with a "no" word in it: `understand` already refuses those, and an
    /// outside opinion must not be able to turn them into a yes.
    pub fn wants_second_opinion(&self, heard: &str) -> bool {
        if heard.trim().is_empty() || match_yes_no(heard) == Answer::No {
            return false;
        }
        match self {
            Prompt::Question { mode: QuestionMode::Single, options, .. } => options.len() >= 2,
            Prompt::Question { .. } => false,
            _ => true,
        }
    }

    /// The options to send with a second-opinion request; `None` for a yes/no prompt.
    pub fn interpret_options(&self) -> Option<&[Choice]> {
        match self {
            Prompt::Question { options, .. } if !options.is_empty() => Some(options),
            _ => None,
        }
    }

    /// The backend's reading (`{"answer": "yes" | "no" | "option" | "unclear", "value"?}`) as an
    /// Answer. Only accepts what fits this prompt: yes/no for approvals, one of this prompt's own
    /// option values for a question. Anything else stays Unclear.
    pub fn answer_from_interpretation(&self, reply: &Value) -> Answer {
        let kind = reply.get("answer").and_then(Value::as_str).unwrap_or_default();
        match self {
            _ if self.is_yes_no() => match kind {
                "yes" => Answer::Yes,
                "no" => Answer::No,
                _ => Answer::Unclear,
            },
            Prompt::Question { mode: QuestionMode::Single, options, .. } if kind == "option" => {
                let value = reply.get("value").and_then(Value::as_str).unwrap_or_default();
                options.iter().find(|o| o.value == value).map_or(Answer::Unclear, |o| Answer::Text(o.value.clone()))
            }
            _ => Answer::Unclear,
        }
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
    /// "Pause", "next", "louder": done on this computer at once, with no trip to Aloe.
    Media(MediaCommand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCommand {
    Pause,
    Play,
    Next,
    Previous,
    VolumeUp,
    VolumeDown,
    Mute,
    Unmute,
}

impl MediaCommand {
    /// The media key that does it (media.rs `press_media_key`).
    pub fn key(self) -> &'static str {
        match self {
            MediaCommand::Pause | MediaCommand::Play => "play_pause",
            MediaCommand::Next => "next",
            MediaCommand::Previous => "previous",
            MediaCommand::VolumeUp => "volume_up",
            MediaCommand::VolumeDown => "volume_down",
            MediaCommand::Mute | MediaCommand::Unmute => "mute",
        }
    }

    /// A volume press is about 2%; five is a change you can hear.
    pub fn presses(self) -> u64 {
        match self {
            MediaCommand::VolumeUp | MediaCommand::VolumeDown => 5,
            _ => 1,
        }
    }

    /// What the panel shows, in place of an answer.
    pub fn label(self) -> &'static str {
        match self {
            MediaCommand::Pause => "Paused",
            MediaCommand::Play => "Playing",
            MediaCommand::Next => "Next",
            MediaCommand::Previous => "Previous",
            MediaCommand::VolumeUp => "Louder",
            MediaCommand::VolumeDown => "Quieter",
            MediaCommand::Mute => "Muted",
            MediaCommand::Unmute => "Unmuted",
        }
    }
}

pub fn local_command(heard: &str) -> Option<LocalCommand> {
    let said = words(heard).join(" ");
    let said = said.trim_start_matches("aloe ").trim_start_matches("please ").trim_end_matches(" please");
    match said {
        "repeat" | "repeat that" | "say that again" | "say it again" | "what did you say" | "come again" | "again" => Some(LocalCommand::Repeat),
        "stop" | "stop talking" | "be quiet" | "quiet" | "shush" | "shut up" | "never mind" | "nevermind" => Some(LocalCommand::Quiet),
        "slower" | "speak slower" | "talk slower" | "slow down" | "speak more slowly" | "more slowly" => Some(LocalCommand::Slower),
        "faster" | "speak faster" | "talk faster" | "speed up" | "speak more quickly" | "more quickly" => Some(LocalCommand::Faster),
        _ => media_command(said).map(LocalCommand::Media),
    }
}

/// The whole of what was said has to be one of these: "pause" is, "pause the download" is not, and
/// goes to Aloe like anything else. "Stop" is not here either: it stops Aloe talking.
fn media_command(said: &str) -> Option<MediaCommand> {
    let said = said.trim_end_matches(" please").trim_end_matches(" for me");
    Some(match said {
        "pause" | "pause it" | "pause that" | "pause the music" | "pause the song" | "pause the video" | "pause music" | "stop the music" | "stop the song" => MediaCommand::Pause,
        "play" | "resume" | "unpause" | "continue playing" | "resume the music" | "play the music" | "resume playing" | "keep playing" => MediaCommand::Play,
        "next" | "skip" | "next song" | "next track" | "skip this" | "skip this song" | "skip this one" | "skip song" | "skip track" | "next one" | "play the next song" => MediaCommand::Next,
        "previous" | "previous song" | "previous track" | "go back a song" | "last song" | "play the previous song" | "back a song" => MediaCommand::Previous,
        "louder" | "volume up" | "turn it up" | "turn up the volume" | "turn the volume up" | "make it louder" | "a bit louder" => MediaCommand::VolumeUp,
        "quieter" | "volume down" | "turn it down" | "turn down the volume" | "turn the volume down" | "make it quieter" | "a bit quieter" | "softer" => MediaCommand::VolumeDown,
        "mute" | "mute it" | "mute that" | "mute the sound" | "mute the volume" => MediaCommand::Mute,
        "unmute" | "unmute it" | "unmute the sound" | "sound on" => MediaCommand::Unmute,
        _ => return None,
    })
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
    fn second_opinion_only_where_it_can_help() {
        let approval = Prompt::CommandApproval { job_id: "j".into(), command: "npm test".into(), reason: String::new() };
        assert!(approval.wants_second_opinion("sounds good to me"));
        assert!(!approval.wants_second_opinion("hmm, not now"), "a no word is final");
        assert!(!approval.wants_second_opinion("  "));
        let single = Prompt::Question { id: "q".into(), question: "Which?".into(), mode: QuestionMode::Single, options: options(&["Downloads", "Desktop"]) };
        assert!(single.wants_second_opinion("the latter"));
        let free = Prompt::Question { id: "q".into(), question: "Name?".into(), mode: QuestionMode::FreeText, options: Vec::new() };
        assert!(!free.wants_second_opinion("Priya"));
        let multi = Prompt::Question { id: "q".into(), question: "Which?".into(), mode: QuestionMode::Multi, options: options(&["A", "B"]) };
        assert!(!multi.wants_second_opinion("both I guess"));
    }

    #[test]
    fn interpretation_must_fit_the_prompt() {
        let approval = Prompt::ConnectorApproval { id: "a".into(), summary: "Send it.".into() };
        assert_eq!(approval.answer_from_interpretation(&json!({ "answer": "yes" })), Answer::Yes);
        assert_eq!(approval.answer_from_interpretation(&json!({ "answer": "no" })), Answer::No);
        assert_eq!(approval.answer_from_interpretation(&json!({ "answer": "option", "value": "x" })), Answer::Unclear);
        assert_eq!(approval.answer_from_interpretation(&json!({ "answer": "unclear" })), Answer::Unclear);

        let single = Prompt::Question { id: "q".into(), question: "Which?".into(), mode: QuestionMode::Single, options: options(&["Downloads", "Desktop"]) };
        assert_eq!(single.answer_from_interpretation(&json!({ "answer": "option", "value": "desktop" })), Answer::Text("desktop".into()));
        assert_eq!(single.answer_from_interpretation(&json!({ "answer": "option", "value": "C:/elsewhere" })), Answer::Unclear, "only this prompt's own values");
        assert_eq!(single.answer_from_interpretation(&json!({ "answer": "yes" })), Answer::Unclear);
        assert_eq!(single.interpret_options().map(|o| o.len()), Some(2));
        assert!(approval.interpret_options().is_none());
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

    #[test]
    fn media_commands_run_here_only_when_they_are_the_whole_sentence() {
        let media = |heard: &str| match local_command(heard) {
            Some(LocalCommand::Media(command)) => Some(command),
            _ => None,
        };
        assert_eq!(media("Pause."), Some(MediaCommand::Pause));
        assert_eq!(media("Aloe, pause the music please"), Some(MediaCommand::Pause));
        assert_eq!(media("Stop the music"), Some(MediaCommand::Pause));
        assert_eq!(media("Resume"), Some(MediaCommand::Play));
        assert_eq!(media("Next song!"), Some(MediaCommand::Next));
        assert_eq!(media("skip this one"), Some(MediaCommand::Next));
        assert_eq!(media("Previous track"), Some(MediaCommand::Previous));
        assert_eq!(media("Turn it up"), Some(MediaCommand::VolumeUp));
        assert_eq!(media("a bit quieter please"), Some(MediaCommand::VolumeDown));
        assert_eq!(media("Mute"), Some(MediaCommand::Mute));
        assert_eq!(media("Unmute"), Some(MediaCommand::Unmute));
        // "Stop" alone still stops Aloe talking, and anything longer is a request for Aloe.
        assert_eq!(local_command("Stop"), Some(LocalCommand::Quiet));
        assert_eq!(media("pause the download in Chrome"), None);
        assert_eq!(media("play Blinding Lights"), None);
        assert_eq!(media("skip to the part about pricing"), None);
        assert_eq!(media("turn the volume up to fifty"), None);
        assert_eq!(media("next slide"), None);
    }

    #[test]
    fn a_volume_command_presses_enough_to_hear() {
        assert_eq!(MediaCommand::VolumeUp.presses(), 5);
        assert_eq!(MediaCommand::Next.presses(), 1);
        assert_eq!(MediaCommand::Pause.key(), "play_pause");
    }
}

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpeechPhase {
    #[default]
    Settled,
    Listening,
    Transcribing,
    Sent,
    Working,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimationSchedule {
    Idle,
    Animate,
    WakeAt(Instant),
}

impl SpeechPhase {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Settled => "\u{f303}",
            Self::Listening => "\u{f036c}",
            Self::Transcribing => "\u{f0f2d}",
            Self::Sent => "\u{f1d8}",
            Self::Working => "\u{ec20}",
        }
    }

    /// Scale each glyph so its visible height matches Arch in the bundled Nerd Font.
    pub fn icon_size(self) -> f32 {
        let glyph_height = match self {
            Self::Settled | Self::Transcribing => 570.,
            Self::Listening => 816.,
            Self::Sent => 615.,
            Self::Working => 708.,
        };
        32. * 570. / glyph_height
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Settled => "Open application launcher",
            Self::Listening => "Listening",
            Self::Transcribing => "Transcribing",
            Self::Sent => "Sent",
            Self::Working => "Agent working",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "event",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SpeechEvent {
    Phase(SpeechPhase),
    Transcript(String),
    Response(String),
}

impl SpeechEvent {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Self::Transcript(text) | Self::Response(text) = self {
            anyhow::ensure!(!text.trim().is_empty(), "speech text must not be empty");
            anyhow::ensure!(
                text.chars().count() <= 1000,
                "speech text exceeds 1000 characters"
            );
            anyhow::ensure!(
                !text
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t'),
                "speech text contains control characters"
            );
        }
        Ok(())
    }
}

pub struct SpeechBubble {
    pub id: u64,
    pub text: String,
    pub is_response: bool,
    created: Instant,
    expires_at: Option<Instant>,
}

impl SpeechBubble {
    fn new(id: u64, text: String, is_response: bool, now: Instant) -> Self {
        Self {
            id,
            text,
            is_response,
            created: now,
            expires_at: Some(now + Duration::from_secs(30)),
        }
    }

    pub fn opacity(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.created).as_secs_f32();
        let remaining = self.expires_at.map_or(f32::INFINITY, |deadline| {
            deadline.saturating_duration_since(now).as_secs_f32()
        });
        (elapsed / 0.18).min(remaining / 0.45).clamp(0., 1.)
    }

    pub fn rise_offset(&self, now: Instant) -> f32 {
        self.expires_at.map_or(0., |deadline| {
            let remaining = deadline.saturating_duration_since(now).as_secs_f32();
            -14. * (1. - remaining / 0.55).clamp(0., 1.)
        })
    }

    pub fn entry_progress(&self, now: Instant) -> f32 {
        let progress =
            (now.saturating_duration_since(self.created).as_secs_f32() / 0.35).clamp(0., 1.);
        1. - (1. - progress).powi(3)
    }

    pub fn expired(&self, now: Instant) -> bool {
        self.expires_at.is_some_and(|deadline| now >= deadline)
    }
}

#[derive(Default)]
pub struct SpeechUi {
    pub phase: SpeechPhase,
    pub messages: Vec<SpeechBubble>,
    next_id: u64,
}

impl gpui::Global for SpeechUi {}

impl SpeechUi {
    pub fn animation_schedule(&self, now: Instant, reduced_motion: bool) -> AnimationSchedule {
        if !reduced_motion
            && matches!(
                self.phase,
                SpeechPhase::Listening | SpeechPhase::Transcribing | SpeechPhase::Working
            )
        {
            return AnimationSchedule::Animate;
        }
        let mut deadline = None;
        for bubble in self.messages.iter().filter(|bubble| !bubble.expired(now)) {
            if !reduced_motion
                && (now.saturating_duration_since(bubble.created) < Duration::from_millis(350)
                    || bubble.expires_at.is_some_and(|expiry| {
                        expiry.saturating_duration_since(now) <= Duration::from_millis(550)
                    }))
            {
                return AnimationSchedule::Animate;
            }
            if let Some(expiry) = bubble.expires_at {
                let wake = if reduced_motion {
                    expiry
                } else {
                    expiry - Duration::from_millis(550)
                };
                deadline = Some(deadline.map_or(wake, |earliest: Instant| earliest.min(wake)));
            }
        }
        deadline.map_or(AnimationSchedule::Idle, AnimationSchedule::WakeAt)
    }

    pub fn apply(&mut self, event: SpeechEvent, now: Instant) {
        self.messages.retain(|bubble| !bubble.expired(now));
        match event {
            SpeechEvent::Phase(phase) => {
                self.phase = phase;
                self.extend_lifetime(now);
            }
            SpeechEvent::Transcript(text) => self.push(text, false, now),
            SpeechEvent::Response(text) => self.push(text, true, now),
        }
    }

    fn push(&mut self, text: String, is_response: bool, now: Instant) {
        if self.messages.len() == 8 {
            self.messages.remove(0);
        }
        self.messages
            .push(SpeechBubble::new(self.next_id, text, is_response, now));
        self.next_id += 1;
        self.extend_lifetime(now);
    }

    pub fn dismiss(&mut self, id: u64) {
        self.messages.retain(|bubble| bubble.id != id);
    }

    fn extend_lifetime(&mut self, now: Instant) {
        for bubble in &mut self.messages {
            bubble.expires_at =
                (self.phase == SpeechPhase::Settled).then(|| now + Duration::from_secs(30));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_bubble_sleeps_until_exit_and_respects_reduced_motion() {
        let now = Instant::now();
        let mut ui = SpeechUi::default();
        ui.apply(SpeechEvent::Response("Done.".into()), now);
        assert_eq!(
            ui.animation_schedule(now, false),
            AnimationSchedule::Animate
        );
        assert_eq!(
            ui.animation_schedule(now + Duration::from_secs(1), false),
            AnimationSchedule::WakeAt(now + Duration::from_millis(29450))
        );
        assert_eq!(
            ui.animation_schedule(now + Duration::from_millis(29600), false),
            AnimationSchedule::Animate
        );
        assert_eq!(
            ui.animation_schedule(now + Duration::from_secs(30), false),
            AnimationSchedule::Idle
        );
        assert_eq!(
            ui.animation_schedule(now, true),
            AnimationSchedule::WakeAt(now + Duration::from_secs(30))
        );
        ui.apply(SpeechEvent::Phase(SpeechPhase::Working), now);
        assert_eq!(
            ui.animation_schedule(now + Duration::from_secs(2), false),
            AnimationSchedule::Animate
        );
        assert_eq!(
            ui.animation_schedule(now + Duration::from_secs(2), true),
            AnimationSchedule::Idle
        );
    }

    #[test]
    fn visible_conversation_retains_only_the_latest_eight_messages() {
        let now = Instant::now();
        let mut ui = SpeechUi::default();
        for message in 0..10 {
            ui.apply(SpeechEvent::Transcript(message.to_string()), now);
        }
        assert_eq!(
            ui.messages
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            ["2", "3", "4", "5", "6", "7", "8", "9"]
        );
    }

    #[test]
    fn follow_up_preserves_the_conversation_in_order() {
        let now = Instant::now();
        let mut ui = SpeechUi::default();
        ui.apply(SpeechEvent::Transcript("Open my project.".into()), now);
        ui.apply(
            SpeechEvent::Response("Your project is open.".into()),
            now + Duration::from_secs(1),
        );
        ui.apply(
            SpeechEvent::Phase(SpeechPhase::Settled),
            now + Duration::from_secs(2),
        );
        ui.apply(
            SpeechEvent::Phase(SpeechPhase::Listening),
            now + Duration::from_secs(10),
        );
        assert_eq!(
            ui.messages
                .iter()
                .map(|bubble| bubble.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Open my project.", "Your project is open."]
        );
        ui.apply(
            SpeechEvent::Transcript("Open its README.".into()),
            now + Duration::from_secs(11),
        );
        ui.apply(
            SpeechEvent::Response("The README is open.".into()),
            now + Duration::from_secs(12),
        );
        assert_eq!(
            ui.messages
                .iter()
                .map(|bubble| (bubble.text.as_str(), bubble.is_response))
                .collect::<Vec<_>>(),
            vec![
                ("Open my project.", false),
                ("Your project is open.", true),
                ("Open its README.", false),
                ("The README is open.", true)
            ]
        );
        ui.apply(
            SpeechEvent::Phase(SpeechPhase::Settled),
            now + Duration::from_secs(12),
        );
        assert!(
            ui.messages
                .iter()
                .all(|bubble| !bubble.expired(now + Duration::from_secs(41)))
        );
        assert!(
            ui.messages
                .iter()
                .all(|bubble| bubble.expired(now + Duration::from_secs(42)))
        );
    }

    #[test]
    fn completion_restores_arch_without_shortening_message_lifetime() {
        let now = Instant::now();
        let mut ui = SpeechUi::default();
        ui.apply(SpeechEvent::Transcript("Open my project.".into()), now);
        ui.apply(
            SpeechEvent::Phase(SpeechPhase::Settled),
            now + Duration::from_secs(1),
        );
        assert_eq!(ui.phase.glyph(), "\u{f303}");
        assert_eq!(ui.messages[0].opacity(now + Duration::from_secs(30)), 1.);
        assert!(ui.messages[0].expired(now + Duration::from_secs(31)));
    }

    #[test]
    fn working_and_follow_up_recording_pause_dismissal() {
        let now = Instant::now();
        let mut ui = SpeechUi::default();
        ui.apply(SpeechEvent::Transcript("Investigate this.".into()), now);
        ui.apply(SpeechEvent::Phase(SpeechPhase::Working), now);
        assert_eq!(ui.messages[0].opacity(now + Duration::from_secs(120)), 1.);
        assert!(!ui.messages[0].expired(now + Duration::from_secs(120)));
        ui.apply(
            SpeechEvent::Phase(SpeechPhase::Settled),
            now + Duration::from_secs(120),
        );
        ui.apply(
            SpeechEvent::Phase(SpeechPhase::Listening),
            now + Duration::from_secs(140),
        );
        assert!(!ui.messages[0].expired(now + Duration::from_secs(180)));
        ui.apply(
            SpeechEvent::Phase(SpeechPhase::Settled),
            now + Duration::from_secs(180),
        );
        assert!(!ui.messages[0].expired(now + Duration::from_secs(209)));
        assert!(ui.messages[0].expired(now + Duration::from_secs(210)));
    }

    #[test]
    fn dismissing_one_message_preserves_the_rest() {
        let now = Instant::now();
        let mut ui = SpeechUi::default();
        ui.apply(SpeechEvent::Transcript("Request".into()), now);
        ui.apply(SpeechEvent::Response("Reply".into()), now);
        ui.dismiss(ui.messages[0].id);
        assert_eq!(
            ui.messages
                .iter()
                .map(|bubble| bubble.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Reply"]
        );
        ui.apply(SpeechEvent::Transcript("Follow up".into()), now);
        assert_ne!(ui.messages[0].id, ui.messages[1].id);
    }

    #[test]
    fn expired_history_is_removed_before_another_message() {
        let now = Instant::now();
        let mut ui = SpeechUi::default();
        ui.apply(SpeechEvent::Response("Old reply.".into()), now);
        ui.apply(
            SpeechEvent::Transcript("New request.".into()),
            now + Duration::from_secs(31),
        );
        assert_eq!(
            ui.messages
                .iter()
                .map(|bubble| bubble.text.as_str())
                .collect::<Vec<_>>(),
            vec!["New request."]
        );
    }
}

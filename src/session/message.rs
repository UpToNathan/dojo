//! Greeting and farewell selection. Pools and rules: `.scratch/lockscreen/greetings.md`.
//!
//! Selection is a pure function of the occasion, a [`Context`] and a seed.
//! The caller supplies the clock, the lock duration and the last line shown.

use std::time::Duration;

use super::{Occasion, Reason};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeOfDay {
    Morning,
    Afternoon,
    Evening,
    Night,
}

impl TimeOfDay {
    /// Morning 05-12, afternoon 12-17, evening 17-22, night 22-05.
    pub fn from_hour(hour: u8) -> Self {
        match hour % 24 {
            5..=11 => Self::Morning,
            12..=16 => Self::Afternoon,
            17..=21 => Self::Evening,
            _ => Self::Night,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    Any,
    TimeOfDay(TimeOfDay),
    LockedUnder2Min,
    LockedOver8h,
    FreshBoot,
    Reason(Reason),
}

const SHORT_LOCK: Duration = Duration::from_secs(2 * 60);
const LONG_LOCK: Duration = Duration::from_secs(8 * 60 * 60);

impl Condition {
    /// Lock duration, boot and reason outrank time of day, which outranks Any.
    fn specificity(self) -> u8 {
        match self {
            Self::Any => 0,
            Self::TimeOfDay(_) => 1,
            Self::LockedUnder2Min | Self::LockedOver8h | Self::FreshBoot | Self::Reason(_) => 2,
        }
    }

    fn holds(self, context: &Context) -> bool {
        match self {
            Self::Any => true,
            Self::TimeOfDay(part) => TimeOfDay::from_hour(context.hour) == part,
            Self::LockedUnder2Min => context.locked_for.is_some_and(|age| age < SHORT_LOCK),
            Self::LockedOver8h => context.locked_for.is_some_and(|age| age >= LONG_LOCK),
            Self::FreshBoot => context.fresh_boot,
            Self::Reason(reason) => context.reason == Some(reason),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub template: &'static str,
    pub when: Condition,
}

const fn any(template: &'static str) -> Entry {
    Entry {
        template,
        when: Condition::Any,
    }
}

const fn at(part: TimeOfDay, template: &'static str) -> Entry {
    Entry {
        template,
        when: Condition::TimeOfDay(part),
    }
}

const fn when(condition: Condition, template: &'static str) -> Entry {
    Entry {
        template,
        when: condition,
    }
}

use TimeOfDay::{Afternoon, Evening, Morning, Night};

const GREETING: &[Entry] = &[
    any("Welcome back, {name}"),
    any("Onwards, {name}"),
    any("Back to it, {name}"),
    any("Step onto the mat, {name}"),
    any("The dojo is yours, {name}"),
    any("Hajime, {name}"),
    any("Bow in, {name}"),
    any("Ready when you are, {name}"),
    any("Carry on, {name}"),
    at(Morning, "Good morning, {name}"),
    at(Morning, "Early start, {name}"),
    at(Morning, "A fresh mat, {name}"),
    at(Afternoon, "Good afternoon, {name}"),
    at(Afternoon, "Afternoon session, {name}"),
    at(Evening, "Good evening, {name}"),
    at(Evening, "Evening practice, {name}"),
    at(Night, "Still going, {name}?"),
    at(Night, "Late session, {name}"),
    at(Night, "Easy does it, {name}"),
    when(Condition::LockedUnder2Min, "That was quick, {name}"),
    when(
        Condition::LockedUnder2Min,
        "Right where you left it, {name}",
    ),
    when(Condition::LockedUnder2Min, "Back already, {name}"),
    when(Condition::LockedOver8h, "A new day, {name}"),
    when(Condition::LockedOver8h, "Long time, {name}"),
    when(Condition::LockedOver8h, "Rested, {name}?"),
];

const LOGIN_BOOT: &[Entry] = &[
    when(Condition::FreshBoot, "Good to see you, {name}"),
    when(Condition::FreshBoot, "Everything is ready, {name}"),
];

const LOCK: &[Entry] = &[
    any("Securing the perimeter"),
    any("Goodbye, {name}"),
    any("Locking the dojo"),
    any("The mat is rolled up"),
    any("Until next time, {name}"),
    any("Door's closed, {name}"),
    any("Perimeter secure"),
    at(Night, "Rest well, {name}"),
    at(Night, "Sleep well, {name}"),
    at(Night, "Lights out, {name}"),
    at(Evening, "Good evening, {name}"),
    at(Evening, "Enjoy the evening, {name}"),
    when(Condition::Reason(Reason::Idle), "Stepping away, {name}?"),
    when(Condition::Reason(Reason::Idle), "The dojo keeps watch"),
    when(Condition::Reason(Reason::Suspend), "Zzzz"),
    when(Condition::Reason(Reason::Suspend), "See you on waking"),
];

const LOGOUT: &[Entry] = &[
    any("Until next time, {name}"),
    any("See you soon, {name}"),
    any("Bow out, {name}"),
    any("Logging out"),
];

const SUSPEND: &[Entry] = &[any("Zzzz"), any("See you on waking")];

const HIBERNATE: &[Entry] = &[
    any("Zzzz"),
    any("Deep sleep, {name}"),
    any("See you on waking"),
];

const SHUTDOWN: &[Entry] = &[
    any("Powering down"),
    any("Closing the dojo"),
    any("Lights out, {name}"),
    at(Night, "Goodnight, {name}"),
];

const RESTART: &[Entry] = &[
    any("Back in a moment, {name}"),
    any("Be right back"),
    any("Taking a breath"),
];

/// Every entry that can be shown for an occasion. Login shares the greeting pool.
pub fn pools(occasion: Occasion) -> &'static [&'static [Entry]] {
    match occasion {
        Occasion::Unlock => &[GREETING],
        Occasion::Login => &[GREETING, LOGIN_BOOT],
        Occasion::Lock => &[LOCK],
        Occasion::Logout => &[LOGOUT],
        Occasion::Suspend => &[SUSPEND],
        Occasion::Hibernate => &[HIBERNATE],
        Occasion::Shutdown => &[SHUTDOWN],
        Occasion::Restart => &[RESTART],
    }
}

#[derive(Clone, Debug, Default)]
pub struct Context {
    /// Local hour of day, 0 to 23.
    pub hour: u8,
    /// How long the session was locked. Only meaningful for Unlock.
    pub locked_for: Option<Duration>,
    pub fresh_boot: bool,
    pub reason: Option<Reason>,
    /// The last line shown for this occasion.
    pub previous: Option<String>,
    pub seed: u64,
}

/// splitmix64, so a seed pins a selection without a random-number crate.
struct Mixer(u64);

impl Mixer {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

fn render(entry: &Entry, name: &str) -> String {
    entry.template.replace("{name}", name)
}

/// Entries whose condition holds, in table order.
pub fn eligible(occasion: Occasion, context: &Context) -> Vec<&'static Entry> {
    pools(occasion)
        .iter()
        .flat_map(|pool| pool.iter())
        .filter(|entry| entry.when.holds(context))
        .collect()
}

/// The line to show: most specific tier first (chosen two times in three), never
/// the previous line unless it is the only choice, `{name}` filled in.
///
/// The returned text for a given `(occasion, context, name)` is fixed by `context.seed`.
pub fn select(occasion: Occasion, context: &Context, name: &str) -> String {
    choose(eligible(occasion, context), context, name)
}

fn choose(mut candidates: Vec<&Entry>, context: &Context, name: &str) -> String {
    if candidates.len() > 1 {
        let fresh: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|entry| context.previous.as_deref() != Some(render(entry, name).as_str()))
            .collect();
        if !fresh.is_empty() {
            candidates = fresh;
        }
    }
    let tier = |level: u8| -> Vec<&Entry> {
        candidates
            .iter()
            .copied()
            .filter(|entry| entry.when.specificity() == level)
            .collect()
    };
    let mut mixer = Mixer(context.seed);
    for level in [2, 1] {
        let specific = tier(level);
        if !specific.is_empty() && mixer.below(3) < 2 {
            return render(specific[mixer.below(specific.len())], name);
        }
    }
    let plain = tier(0);
    let pool = if plain.is_empty() { candidates } else { plain };
    render(pool[mixer.below(pool.len())], name)
}

/// The first word of the GECOS full name, else the login with a capital letter.
pub fn display_name(gecos: Option<&str>, login: &str) -> String {
    let full_name = gecos
        .and_then(|field| field.split(',').next())
        .and_then(|name| name.split_whitespace().next());
    if let Some(first) = full_name {
        return first.to_owned();
    }
    let mut letters = login.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(hour: u8, seed: u64) -> Context {
        Context {
            hour,
            seed,
            ..Context::default()
        }
    }

    #[test]
    fn splitmix64_matches_the_reference_vector() {
        let mut mixer = Mixer(0);
        assert_eq!(mixer.next(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(mixer.next(), 0x6e78_9e6a_a1b9_65f4);
    }

    #[test]
    fn time_of_day_boundaries() {
        for (hour, part) in [
            (4, Night),
            (5, Morning),
            (11, Morning),
            (12, Afternoon),
            (16, Afternoon),
            (17, Evening),
            (21, Evening),
            (22, Night),
            (0, Night),
        ] {
            assert_eq!(TimeOfDay::from_hour(hour), part, "hour {hour}");
        }
    }

    #[test]
    fn every_occasion_has_an_always_eligible_line_at_every_hour() {
        for occasion in Occasion::ALL {
            for hour in 0..24 {
                let lines = eligible(occasion, &context(hour, 0));
                assert!(
                    lines.iter().any(|entry| entry.when == Condition::Any),
                    "{occasion:?} at {hour}"
                );
                for seed in 0..50 {
                    let line = select(occasion, &context(hour, seed), "Alyssa");
                    assert!(!line.is_empty(), "{occasion:?} at {hour} seed {seed}");
                }
            }
        }
    }

    #[test]
    fn every_template_renders_with_the_name_and_no_braces() {
        for occasion in Occasion::ALL {
            for entry in pools(occasion).iter().flat_map(|pool| pool.iter()) {
                let line = render(entry, "Alyssa");
                assert!(!line.contains('{') && !line.contains('}'), "{line}");
                assert_eq!(
                    line.contains("Alyssa"),
                    entry.template.contains("{name}"),
                    "{line}"
                );
                assert!(!line.contains('!'), "{line}");
            }
        }
    }

    #[test]
    fn never_repeats_the_previous_line() {
        for occasion in Occasion::ALL {
            for hour in [3, 9, 14, 19, 23] {
                let mut previous = None;
                for seed in 0..300 {
                    let ctx = Context {
                        previous: previous.clone(),
                        ..context(hour, seed)
                    };
                    let line = select(occasion, &ctx, "Alyssa");
                    assert_ne!(Some(&line), previous.as_ref(), "{occasion:?} seed {seed}");
                    previous = Some(line);
                }
            }
        }
    }

    #[test]
    fn a_lone_eligible_entry_may_repeat_the_previous_line() {
        let only = Entry {
            template: "Hajime, {name}",
            when: Condition::Any,
        };
        let ctx = Context {
            previous: Some("Hajime, Alyssa".into()),
            ..context(10, 1)
        };
        assert_eq!(choose(vec![&only], &ctx, "Alyssa"), "Hajime, Alyssa");
    }

    #[test]
    fn specific_lines_are_chosen_about_two_times_in_three() {
        let mut specific = 0;
        for seed in 0..3000 {
            let line = select(Occasion::Unlock, &context(23, seed), "N");
            if ["Still going, N?", "Late session, N", "Easy does it, N"].contains(&line.as_str()) {
                specific += 1;
            }
        }
        assert!((1900..2100).contains(&specific), "{specific} of 3000");
    }

    #[test]
    fn lock_duration_outranks_time_of_day() {
        let quick = Context {
            locked_for: Some(Duration::from_secs(30)),
            ..context(23, 0)
        };
        let mut duration = 0;
        let mut night = 0;
        for seed in 0..3000 {
            let line = select(
                Occasion::Unlock,
                &Context {
                    seed,
                    ..quick.clone()
                },
                "N",
            );
            if [
                "That was quick, N",
                "Right where you left it, N",
                "Back already, N",
            ]
            .contains(&line.as_str())
            {
                duration += 1;
            } else if ["Still going, N?", "Late session, N", "Easy does it, N"]
                .contains(&line.as_str())
            {
                night += 1;
            }
        }
        assert!((1900..2100).contains(&duration), "{duration}");
        assert!((600..740).contains(&night), "{night}");
    }

    #[test]
    fn duration_thresholds() {
        let at = |secs| Context {
            locked_for: Some(Duration::from_secs(secs)),
            ..context(10, 0)
        };
        let has = |ctx: &Context, text: &str| {
            eligible(Occasion::Unlock, ctx)
                .iter()
                .any(|entry| entry.template == text)
        };
        assert!(has(&at(119), "Back already, {name}"));
        assert!(!has(&at(120), "Back already, {name}"));
        assert!(!has(&at(8 * 3600 - 1), "A new day, {name}"));
        assert!(has(&at(8 * 3600), "A new day, {name}"));
        assert!(!has(&context(10, 0), "A new day, {name}"));
    }

    #[test]
    fn fresh_boot_lines_only_appear_at_login_after_a_boot() {
        let boot = Context {
            fresh_boot: true,
            ..context(10, 0)
        };
        let boot_line = |ctx: &Context, occasion| {
            eligible(occasion, ctx)
                .iter()
                .any(|entry| entry.template == "Good to see you, {name}")
        };
        assert!(boot_line(&boot, Occasion::Login));
        assert!(!boot_line(&context(10, 0), Occasion::Login));
        assert!(!boot_line(&boot, Occasion::Unlock));
    }

    #[test]
    fn reason_pool_is_eligible_only_with_a_matching_reason() {
        let texts = |ctx: &Context| -> Vec<&str> {
            eligible(Occasion::Lock, ctx)
                .iter()
                .map(|entry| entry.template)
                .collect()
        };
        let idle = Context {
            reason: Some(Reason::Idle),
            ..context(10, 0)
        };
        assert!(texts(&idle).contains(&"The dojo keeps watch"));
        assert!(!texts(&idle).contains(&"Zzzz"));
        let manual = Context {
            reason: Some(Reason::Manual),
            ..context(10, 0)
        };
        assert!(!texts(&manual).contains(&"The dojo keeps watch"));
        assert!(!texts(&context(10, 0)).contains(&"The dojo keeps watch"));
    }

    #[test]
    fn night_only_lines_stay_out_of_the_day() {
        let day = eligible(Occasion::Shutdown, &context(14, 0));
        assert!(
            !day.iter()
                .any(|entry| entry.template.starts_with("Goodnight"))
        );
        let night = eligible(Occasion::Shutdown, &context(23, 0));
        assert!(
            night
                .iter()
                .any(|entry| entry.template.starts_with("Goodnight"))
        );
    }

    #[test]
    fn selection_is_pinned_for_fixed_seeds() {
        let lines: Vec<String> = (0..4)
            .map(|seed| select(Occasion::Unlock, &context(23, seed), "Alyssa"))
            .collect();
        assert_eq!(lines, PINNED_UNLOCK_NIGHT);
        let farewells: Vec<String> = (0..4)
            .map(|seed| select(Occasion::Lock, &context(10, seed), "Alyssa"))
            .collect();
        assert_eq!(farewells, PINNED_LOCK_MORNING);
    }

    const PINNED_UNLOCK_NIGHT: [&str; 4] = [
        "Still going, Alyssa?",
        "Ready when you are, Alyssa",
        "Easy does it, Alyssa",
        "Still going, Alyssa?",
    ];
    const PINNED_LOCK_MORNING: [&str; 4] = [
        "Locking the dojo",
        "Locking the dojo",
        "Until next time, Alyssa",
        "Locking the dojo",
    ];

    #[test]
    fn display_name_prefers_the_full_name_first_word() {
        assert_eq!(display_name(Some("Alyssa Park,,,"), "apark"), "Alyssa");
        assert_eq!(display_name(Some("Ada"), "ada"), "Ada");
        assert_eq!(
            display_name(Some("  Grace Hopper , Room 4"), "ghopper"),
            "Grace"
        );
        assert_eq!(display_name(None, "alyssa"), "Alyssa");
        assert_eq!(display_name(Some(""), "alyssa"), "Alyssa");
        assert_eq!(display_name(Some(",,,"), "alyssa"), "Alyssa");
        assert_eq!(display_name(Some("   "), "élodie"), "Élodie");
        assert_eq!(display_name(None, ""), "");
    }
}

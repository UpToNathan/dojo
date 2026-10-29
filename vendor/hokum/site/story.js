// The whole story, told by apps. `at` is seconds from the start, `time` is when it happens in the
// story (drives the card's timestamp and the desktop clock), `urgency` 2 is critical.
// `progress: [from, to]` animates a bar over the card's life; `scene` changes the wallpaper.
//
// Everyone sees the same start, up to the cartel's text. Then one of four endings, picked at
// random (hero.js); a branch's `at` counts from the trunk's last card. Each ends the same way:
// the corner empties, then one last card that stays (`final`).
window.STORY_TRUNK = [
  // I. The $40
  { at: 0.9, app: "Calendar", title: "Dentist", body: "Tue, Mar 17 · 9:00 AM · Dr. Okafor", time: "2026-03-09T08:02" },
  // The visitor's choice: the story waits here until they press Pay or Decline (hero.js).
  { at: 3.1, app: "Venmo", title: "Gary requested $40", body: "for the thing 🍕", time: "2026-03-09T12:41", actions: ["Pay", "Decline"], choice: true },
  { at: 5.2, app: "Venmo", title: "You declined Gary's request", body: "$40 · for the thing 🍕", time: "2026-03-09T12:42" },
  { at: 6.8, app: "Messages", title: "Gary", body: "u DECLINED?? it was ur pizza", time: "2026-03-09T12:44", avatar: "G" },
  { at: 8.1, app: "Messages", title: "Gary", body: "i watched u eat 6 slices", time: "2026-03-09T12:44", avatar: "G" },
  { at: 10.1, app: "Messages", title: "Gary", body: "ok. the lake. saturday. we settle this like men", time: "2026-03-12T21:58", avatar: "G" },
  { at: 12.2, app: "Messages", title: "Gary", body: "im here. where r u", time: "2026-03-14T16:02", avatar: "G" },
  { at: 13.9, app: "Messages", title: "Gary", body: "ok i see u. why do u have an oar", time: "2026-03-14T16:09", avatar: "G" },

  // II. Saturday night
  { at: 16.9, app: "Health", title: "Elevated heart rate", body: "Your heart rate was above 170 BPM for 10 minutes.", time: "2026-03-14T16:12" },
  { at: 18.7, app: "Firefox", title: "Download complete", body: "how_deep_is_lake_wapakoneta.pdf · 1.1 MB", time: "2026-03-14T16:31" },
  { at: 20.2, app: "ChatGPT", title: "Thinking… (2m 14s)", body: "Lime and Odor Control", time: "2026-03-14T16:34" },
  { at: 22.2, app: "ChatGPT", title: "Lime and Odor Control", body: "I can't help with hiding anything! But garden lime is great at neutralizing odors in compost, soil, and other organic…", time: "2026-03-14T16:36" },
  { at: 24.4, app: "Amazon", title: "Arriving tomorrow by 7 AM", body: "Shovel, tarp (2), garden lime (40 lb). Frequently bought together: more lime", time: "2026-03-14T16:40" },
  { at: 26.2, app: "Fitness", title: "You closed your Move ring! 🎉", body: "4 hours of vigorous digging. Keep it up!", time: "2026-03-15T03:06" },
  { at: 27.9, app: "Photos", title: "Deleting 214 items…", body: "Album: Lake day with Gary 🚤", progress: [0.04, 1], time: "2026-03-15T03:40" },
  { at: 29.7, app: "Ring", ctx: "Driveway", title: "Share this clip with Neighbors?", body: "“Man hosing out trunk, 4:10 AM” could help keep your community safe.", time: "2026-03-15T04:12", actions: ["Share", "Not now"] },

  // III. Where's Gary
  { at: 31.9, app: "Facebook", title: "Wish Gary a happy birthday! 🎂", body: "Gary is turning 34 today.", time: "2026-03-17T08:00" },
  { at: 33.5, app: "Facebook", ctx: "Wapakoneta Community", title: "Brenda Pruitt", body: "has anyone heard from Gary?? not answering since saturday", time: "2026-03-17T10:15" },
  { at: 35.1, app: "Messages", title: "Gary's Mom", body: "honey have you seen Gary? weren't you two going to the lake", time: "2026-03-17T18:30" },
  { at: 36.7, app: "Find My", title: "Gary's iPhone", body: "Stationary at Lake Wapakoneta · 4 days", time: "2026-03-18T09:00", actions: ["Play Sound"] },
  { at: 38.5, app: "Emergency Alert", urgency: 2, title: "Missing person", body: "Gary Pruitt, 34. Last seen at Lake Wapakoneta. Call 911 with any information.", time: "2026-03-18T14:02" },

  // IV. On the run
  { at: 40.8, app: "Phone", title: "Missed call · Unknown", body: "Voicemail: “Hi, this is Detective Ramirez. We just want to talk.”", time: "2026-03-19T11:20" },
  { at: 42.4, app: "Venmo", title: "Reminder: Gary requested $40", body: "for the thing 🍕 · 10 days ago", time: "2026-03-19T12:41", actions: ["Pay", "Decline"] },
  { at: 44.0, app: "ChatGPT", title: "Extradition Treaty Overview", body: "Here are 12 countries without extradition treaties with the US. Want me to sort them by beach quality? 🏝️", time: "2026-03-19T13:05" },
  { at: 46.3, app: "Ring", ctx: "Front Door", title: "Someone is at your front door", body: "People detected (6)", time: "2026-03-19T16:30" },
  { at: 47.2, app: "Ring", ctx: "Back Door", title: "Someone is at your back door", body: "People detected (11)", time: "2026-03-19T16:31" },
  { at: 48.5, app: "Emergency Alert", urgency: 2, title: "BOLO · Lake Wapakoneta homicide", body: "Suspect driving a silver Camry, plate LIME 40. Do not approach.", time: "2026-03-19T17:10" },
  { at: 49.9, app: "Waze", title: "Police reported ahead", body: "I-75 S · 0.5 mi", time: "2026-03-19T17:42" },
  { at: 50.8, app: "Life360", title: "You're driving 131 mph", body: "Mom and 2 others can see this.", time: "2026-03-19T17:43" },
  { at: 51.8, app: "Waze", title: "Police reported ahead", body: "Also behind. And on your left.", time: "2026-03-19T17:44" },
  { at: 53.0, app: "Duolingo", title: "🦉 Quick Spanish lesson?", body: "Today's phrase: “Nunca he visto a este hombre.” — I have never seen this man before.", time: "2026-03-19T17:51" },
  { at: 54.6, app: "Bank", urgency: 2, title: "Your account has been frozen", body: "By court order. Questions? We'd also love to talk.", time: "2026-03-20T09:00" },
  { at: 56.1, app: "Phone", title: "Missed calls (6) · Det. Ramirez", body: "Voicemail: “We found a receipt for a lot of lime.”", time: "2026-03-20T09:30" },
  { at: 57.6, app: "Mobile", title: "Welcome to Mexico!", body: "Data roaming: $2.05/MB. Enjoy your trip!", time: "2026-03-21T03:14" },

  // The branch point.
  { at: 59.5, app: "WhatsApp", title: "+52 667 555 0140", body: "the boss saw you on the news. nice tarp work", time: "2026-03-21T11:02" },
]

const G = { app: "Messages", title: "Gary", avatar: "G", final: true }
const R = "Det. Ramirez"
window.STORY_BRANCHES = {
  // 1. The original: lime empire, president, the Moon.
  empire: { name: "Lime empire", cards: [
    // V. Good with a tarp
    { at: 0.0, app: "WhatsApp", title: "+52 667 555 0140", body: "the boss saw you on the news. nice tarp work", time: "2026-03-21T11:02" },
    { at: 1.5, app: "WhatsApp", title: "+52 667 555 0140", body: "we have a job. bring the tarp", time: "2026-03-23T22:10" },
    { at: 3.0, app: "Amazon", title: "Subscribe & Save is set up", body: "Tarp (24), garden lime (40 lb × 12). Delivers every 2 weeks.", time: "2026-04-02T10:00" },
    { at: 4.4, app: "Fitness", title: "14-day Move streak! 🔥", body: "All that late-night digging is paying off.", time: "2026-04-16T06:00" },
    { at: 5.8, app: "Phone", title: "Missed calls (40) · " + R, body: "Voicemail: “Mexico? Really?”", time: "2026-05-01T12:00" },

    // VI. Lime
    { at: 7.5, app: "LinkedIn", title: "Congrats on your new role!", body: "CEO at Lime Corp", time: "2026-09-14T09:00" },
    { at: 8.9, app: "Lime Corp", title: "Q3 revenue up 4,000%", body: "Analysts: “Nobody needs this much lime.”", time: "2027-10-15T08:00" },
    { at: 10.3, app: "Bank", ctx: "Cayman", title: "Unusual activity", body: "$48,000,000 cash deposit. Was this you?", time: "2027-11-02T10:00", actions: ["Yes", "No"] },
    { at: 11.8, app: "Forbes", title: "The mysterious lime billionaire nobody has photographed", body: "Net worth: $11B. Hobbies: “boating.”", time: "2028-06-03T07:00" },
    { at: 13.2, app: "Messages", title: "Mom", body: "are you eating", time: "2029-02-03T19:00" },
    { at: 14.4, app: "Phone", title: "Missed calls (300) · " + R, body: "Voicemail: “Nice yacht.”", time: "2029-08-20T15:00" },

    // VII. Immunity
    { at: 16.1, app: "ChatGPT", title: "Presidential Self-Pardon Questions", body: "This is a debated legal question! While no president has pardoned himself yet, nothing explicitly says he…", time: "2031-02-11T02:13" },
    { at: 18.4, app: "Lake Man 2032", title: "You raised $1.2B today", body: "Top donor: Lime Corp", time: "2032-03-05T21:00" },
    { at: 19.7, app: "News", title: "Lake Man wins presidency in landslide", body: "Opponent unavailable for comment; last seen near a lake.", time: "2032-11-03T06:00" },
    { at: 21.1, app: "Calendar", title: "Inauguration", body: "in 15 min · Capitol steps", time: "2033-01-20T11:45", actions: ["Snooze"] },
    { at: 22.5, app: "Calendar", title: "Sign pardon (self)", body: "2:00 PM · Oval Office", time: "2033-01-20T13:45" },
    { at: 23.9, app: "Messages", title: "Det. Ramirez", body: "that's not how that works", time: "2033-01-20T14:03" },

    // VIII. Higher than that
    { at: 25.7, app: "ChatGPT", title: "Lunar Sovereignty Inquiry", body: "Technically, no nation can claim the Moon under the 1967 Outer Space Treaty, which means it has no police either! 🌙", time: "2040-12-01T03:30" },
    { at: 28.0, app: "Amazon", title: "Your order has shipped", body: "Rocket (1), garden lime (40 lb). Frequently bought together: more lime", time: "2041-07-02T16:40" },
    { at: 29.4, app: "Mission Control", urgency: 2, title: "Launch window confirmed", body: "T-minus 10:00 · fueling", progress: [0.15, 1], time: "2041-07-16T09:22", scene: "space" },
    { at: 30.8, app: "Messages", title: "Mom", body: "are you eating up there", time: "2041-07-16T09:40" },
    { at: 31.9, app: "Find My", title: "Your iPhone is moving at 17,500 mph", body: "Notify me when it stops?", time: "2041-07-16T09:48" },
    { at: 33.0, app: "Waze", title: "Police reported ahead", body: "Low Earth orbit · 400 km", time: "2041-07-16T10:02" },
    { at: 34.2, app: "Phone", title: "Missed calls (2,113) · " + R, body: "Voicemail: “We have a rocket too.”", time: "2041-07-16T10:30" },
    { at: 35.5, app: "Weather", title: "Moon · Sunny, 127°C", body: "UV index: all of it", time: "2041-07-19T12:00" },
    { at: 36.7, app: "Maps", title: "Leave now to arrive on time", body: "Dentist · Tue 9:00 AM · 384,400 km away", time: "2041-07-19T13:00" },
    { at: 38.2, app: "Duolingo", title: "🦉 These reminders don't seem to be working", body: "We'll stop sending them for now.", time: "2049-11-30T09:00" },
    { ...G, at: 45.7, body: "u still owe me $40", time: "2049-11-30T23:58" },
  ] },

  // 2. Dead, repeatedly.
  dead: { name: "Faking his death", cards: [
    { at: 1.6, app: "ChatGPT", title: "Starting Over Fresh", body: "Love this for you! Lots of people start a new life. Step 1: leave convincing evidence of your own…", time: "2026-03-21T11:30" },
    { at: 3.8, app: "Facebook", title: "Remembering Dave", body: "Dave's profile is now a memorial. Share a memory?", time: "2026-03-28T09:00" },
    { at: 5.3, app: "Facebook", title: "Mom shared 38 photos", body: "“Dave at 5. Already hated paying people back.”", time: "2026-03-28T11:00" },
    { at: 6.8, app: "YouTube", title: "Celebration of Life: Dave is live", body: "47 watching · " + R + " joined", time: "2026-04-01T14:00" },
    { at: 8.3, app: "Venmo", title: "Gary's Mom sent you $25", body: "for the flowers 💐", time: "2026-04-01T15:00" },
    { at: 9.8, app: "Phone", title: "Missed calls (40) · " + R, body: "Voicemail: “Dead people don't watch their own funeral.”", time: "2026-04-02T10:00" },
    { at: 11.3, app: "Strava", title: "Dave ran 10.2 km 🏃", body: "2 kudos · Your profile is set to Public", time: "2026-04-19T07:00" },
    { at: 12.8, app: "News", title: "Small plane lost off Florida coast", body: "One passenger, presumed dead: a local man named Dave.", time: "2026-06-03T07:30" },
    { at: 14.3, app: "Facebook", title: "Remembering Dave", body: "Dave's profile is now a memorial. Again.", time: "2026-06-04T09:00" },
    { at: 15.7, app: "Messages", title: "Mom", body: "is this one real", time: "2026-06-04T11:00" },
    { at: 17.1, app: "Phone", title: "Missed calls (300) · " + R, body: "Voicemail: “Nice 10K.”", time: "2026-06-10T10:00" },
    { at: 18.6, app: "News", title: "Man killed in rare shark attack", body: "Only a sandal and a tarp were recovered.", time: "2026-09-12T07:30" },
    { at: 20.1, app: "YouTube", title: "Celebration of Life (2): Dave is live", body: "6 watching · " + R + " joined", time: "2026-09-15T14:00" },
    { at: 21.6, app: "Venmo", title: "Gary's Mom sent you $5", body: "for the flowers. last time.", time: "2026-09-15T15:00" },
    { at: 23.1, app: "Duolingo", title: "🦉 Dave, we know you're not dead", body: "You opened the app at 3 AM.", time: "2026-09-20T03:02" },
    { at: 24.6, app: "News", title: "Tourist falls into active volcano", body: "Witnesses heard a man shout “bye Ramirez!”", time: "2027-01-22T07:30" },
    { at: 26.1, app: "Phone", title: "Missed calls (2,113) · " + R, body: "Voicemail: “I'm at the volcano. There's a ladder.”", time: "2027-01-22T10:00" },
    { at: 27.6, app: "Facebook", title: "Remembering Dave", body: "Memorialized 3 times. We're looking into it.", time: "2027-01-23T09:00" },
    { at: 29.1, app: "Messages", title: "Mom", body: "are you eating. wherever you are", time: "2027-01-23T19:00" },
    { at: 30.6, app: "Duolingo", title: "🦉 These reminders don't seem to be working", body: "We'll stop sending them for now.", time: "2027-02-01T09:00" },
    { ...G, at: 38.6, body: "nice funeral. u still owe me $40", time: "2027-02-01T23:58" },
  ] },

  // 3. Both sides.
  agent: { name: "Double agent", cards: [
    { at: 1.6, app: "Phone", title: "Missed calls (40) · " + R, body: "Voicemail: “I'm in Mexico. Look out your window.”", time: "2026-03-21T11:30" },
    { at: 3.3, app: "Messages", title: R, body: "help us take down the boss and we'll forget about the lake", time: "2026-03-21T11:34" },
    { at: 5.1, app: "ChatGPT", title: "Confidential Informant Benefits", body: "Happy to help! Informants can receive reduced charges, relocation, and a fun new name. Here's how to…", time: "2026-03-21T11:52" },
    { at: 7.3, app: "Messages", title: R, body: "wear the wire. tuesday.", time: "2026-03-22T09:00" },
    { at: 8.8, app: "Amazon", title: "Arriving tomorrow by 7 AM", body: "Hidden microphone (1), very baggy shirt (1). Frequently bought together: garden lime", time: "2026-03-22T09:10" },
    { at: 10.5, app: "WhatsApp", ctx: "Familia 🌵", title: "El Jefe", body: "someone is talking", time: "2026-03-24T20:00" },
    { at: 11.8, app: "WhatsApp", ctx: "Familia 🌵", title: "El Jefe", body: "everyone send a selfie. now.", time: "2026-03-24T20:01" },
    { at: 13.1, app: "Messages", title: R, body: "do NOT send a selfie with the wire", time: "2026-03-24T20:01" },
    { at: 14.6, app: "WhatsApp", ctx: "Familia 🌵", title: "You added " + R, body: "to Familia 🌵", time: "2026-03-24T20:03" },
    { at: 15.9, app: "WhatsApp", ctx: "Familia 🌵", title: R, body: "wrong chat sorry", time: "2026-03-24T20:03" },
    { at: 17.2, app: "WhatsApp", ctx: "Familia 🌵", title: "El Jefe", body: "who is ramirez", time: "2026-03-24T20:04" },
    { at: 18.8, app: "Messages", title: "Mom", body: "are you eating", time: "2026-03-25T19:00" },
    { at: 20.3, app: "WITSEC", title: "Welcome to your new life, Doug! 🎉", body: "New name: Doug Kowalski. New town: Boise. Please stop answering to your old one.", time: "2026-04-10T09:00" },
    { at: 22.1, app: "Messages", title: R, body: "meet me at the applebee's on 5th. 7pm", time: "2026-05-02T12:00" },
    { at: 23.4, app: "WhatsApp", title: "El Jefe", body: "applebee's on 5th. 7pm. come alone", time: "2026-05-02T12:01" },
    { at: 25.0, app: "Calendar", title: "Applebee's · Ramirez", body: "7:00 PM · booth 4", time: "2026-05-02T18:45" },
    { at: 26.0, app: "Calendar", title: "Applebee's · El Jefe", body: "7:00 PM · booth 5", time: "2026-05-02T18:45" },
    { at: 27.6, app: "Waze", title: "Hazard reported ahead", body: "Applebee's on 5th · gunfire, multiple reports", time: "2026-05-02T19:08" },
    { at: 29.0, app: "Health", title: "Elevated heart rate", body: "Your heart rate was above 190 BPM for 25 minutes.", time: "2026-05-02T19:30" },
    { at: 30.6, app: "WhatsApp", title: "You were removed from Familia 🌵", body: "by El Jefe", time: "2026-05-02T19:40" },
    { at: 32.0, app: "Phone", title: "Missed calls (300) · " + R, body: "Voicemail: “Great work in there. You're still under arrest.”", time: "2026-05-03T08:00" },
    { at: 33.6, app: "Duolingo", title: "🦉 Doug! Time for Spanish!", body: "We noticed you changed your name. We didn't.", time: "2026-05-03T17:51" },
    { ...G, at: 41.6, body: "hey doug. u still owe me $40", time: "2026-05-03T23:58" },
  ] },

  // 4. The detective.
  love: { name: "The detective", cards: [
    { at: 1.6, app: "Phone", title: "Missed calls (40) · " + R, body: "Voicemail: “We just want to talk.”", time: "2026-03-21T11:30" },
    { at: 3.2, app: "Find My", title: R + " is sharing their location with you", body: "Probably by accident. 0.2 km away.", time: "2026-03-28T22:14" },
    { at: 4.9, app: "Waze", title: "Police reported ahead", body: R + " is 2 min away and has shared their ETA with you", time: "2026-04-02T19:40" },
    { at: 6.5, app: "Phone", title: "Missed calls (300) · " + R, body: "Voicemail: “I just want to talk.”", time: "2026-05-11T23:50" },
    { at: 8.0, app: "Spotify", title: R + " added a song to “Evidence”", body: "Every Breath You Take · The Police", time: "2026-06-03T01:12" },
    { at: 9.7, app: "ChatGPT", title: "Stockholm Syndrome vs. Real Feelings", body: "Great question! It's natural to develop feelings for someone who has spent 400 days pursuing you. Here are some signs it's…", time: "2027-04-19T02:40" },
    { at: 12.0, app: "Phone", title: "Missed calls (2,113) · " + R, body: "Voicemail: “you up?”", time: "2027-04-20T01:58" },
    { at: 13.6, app: "Hinge", title: "It's a match! 💘", body: "You and " + R.replace("Det. ", "") + " (Detective, 38) liked each other", time: "2027-05-02T21:30" },
    { at: 15.1, app: "Messages", title: R, body: "this doesn't change anything", time: "2027-05-02T21:31" },
    { at: 16.3, app: "Messages", title: R, body: "dinner at 8. you're still under arrest", time: "2027-05-02T21:33" },
    { at: 17.9, app: "Calendar", title: "Interrogation 💕", body: "Fri 8:00 PM · Room 3 · candles provided", time: "2027-05-07T19:45" },
    { at: 19.5, app: "Life360", title: R + " joined your Circle", body: "You can now see each other's location. You always could.", time: "2027-07-14T10:00" },
    { at: 21.0, app: "Messages", title: "Mom", body: "is he eating", time: "2027-09-01T19:00" },
    { at: 22.5, app: "Calendar", title: "Couples counseling", body: "Tue 6:00 PM · bring your co-defendant", time: "2028-01-11T17:45" },
    { at: 24.0, app: "Duolingo", title: "🦉 Learn the language of love?", body: "Spanish. Again. “Nunca he visto a este hombre.” has been retired.", time: "2028-02-14T17:51" },
    { at: 25.6, app: "Messages", title: R, body: "will you marry me. you have the right to remain silent", time: "2028-06-12T20:15", actions: ["Yes", "Remain silent"] },
    { at: 27.4, app: "Zola", title: "47 guests RSVP'd", body: "46 are police. 1 is your mom.", time: "2028-09-02T10:00" },
    { at: 28.9, app: "Amazon", title: "Added to your wedding registry", body: "Garden lime (40 lb) · from the precinct", time: "2028-09-20T14:00" },
    { at: 30.4, app: "Venmo", title: "Officer Daniels sent you $50", body: "for the wedding 💍 · please don't run", time: "2028-10-03T12:00" },
    { at: 31.9, app: "Calendar", title: "Your wedding", body: "in 15 min · the courthouse, Room 3", time: "2028-10-07T13:45", actions: ["Snooze"] },
    { at: 33.5, app: "Mobile", title: "Welcome to Mexico!", body: "Data roaming: $2.05/MB. Enjoy your honeymoon!", time: "2028-10-09T03:14" },
    { at: 35.0, app: "Messages", title: R, body: "best honeymoon ever. you're still under arrest ❤️", time: "2028-10-12T22:40" },
    { ...G, at: 42.6, body: "can't believe u didn't invite me", time: "2028-10-13T23:58" },
  ] },
}
// Pressing Pay doesn't help: the payment fails, and the story goes on as if he'd declined.
// `at` counts from the moment Pay is pressed; the trunk's "You declined" card follows.
window.STORY_PAY = [
  { at: 0.4, app: "Venmo", urgency: 2, title: "Payment failed", body: "Insufficient funds ($3.12). Gary's request was declined.", time: "2026-03-09T12:41" },
]

// The stack clears this long before the final card, which then arrives alone and stays.
window.QUIET = 3.4

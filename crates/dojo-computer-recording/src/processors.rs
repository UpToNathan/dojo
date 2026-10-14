//! Offline GUI-R1 and Aguvis preprocessing of explicitly consented annotated fixtures.
//! Translates MIT cua_bench/processors at the pinned revision in provenance.json.
use crate::{Error, MAX_INPUT_BYTES, scrub_string};
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl BoundingBox {
    fn validate(self) -> Result<Self, Error> {
        if [self.x, self.y, self.width, self.height]
            .iter()
            .any(|n| !n.is_finite() || n.abs() > 1_000_000.0)
            || self.width <= 0.0
            || self.height <= 0.0
        {
            Err(Error::Invalid(
                "non-finite, empty or oversized bounding box",
            ))
        } else {
            Ok(self)
        }
    }
    fn center(self) -> [f64; 2] {
        [self.x + self.width / 2.0, self.y + self.height / 2.0]
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessorInput {
    pub annotated_html: String,
    pub width: u32,
    pub height: u32,
    pub os_type: String,
    pub image_sha256: String,
    pub capture_consent: bool,
    #[serde(default)]
    pub secret_values: Vec<String>,
    /// RGB bytes are optional for GUI-R1; required for Aguvis text-border shrinking.
    #[serde(default)]
    pub rgb_base64: Option<String>,
    /// Required when data-actions contains predefined coordinate actions.
    #[serde(default)]
    pub action_coordinate_space: Option<crate::benchmark::CoordinateSpace>,
}
impl ProcessorInput {
    fn validate(&self) -> Result<(), Error> {
        if !self.capture_consent {
            return Err(Error::Invalid("annotated capture consent required"));
        }
        if self.annotated_html.len() > 256 * 1024
            || self.os_type.len() > 64
            || self.width == 0
            || self.height == 0
            || u64::from(self.width) * u64::from(self.height) > 2_097_152
        {
            return Err(Error::Limit("processor HTML/image bounds"));
        }
        if self.image_sha256.len() != 64
            || !self.image_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::Invalid("processor requires image SHA256"));
        }
        crate::redact_value(&serde_json::Value::Null, &self.secret_values)?;
        Ok(())
    }
    fn rgb(&self) -> Result<Option<RgbImage>, Error> {
        use base64::Engine;
        let Some(encoded) = &self.rgb_base64 else {
            return Ok(None);
        };
        if encoded.len() > MAX_INPUT_BYTES {
            return Err(Error::Limit("RGB bytes"));
        }
        let pixels = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| Error::Invalid("RGB base64"))?;
        if pixels.len() != self.width as usize * self.height as usize * 3 {
            return Err(Error::Invalid("RGB dimensions"));
        }
        Ok(Some(RgbImage {
            width: self.width,
            height: self.height,
            pixels,
        }))
    }
}
#[derive(Clone, Debug)]
struct Target {
    label: String,
    bbox: BoundingBox,
}
fn bounding_box(element: ElementRef<'_>) -> Option<BoundingBox> {
    if element.value().attr("data-bbox-center-hit") != Some("true") {
        return None;
    }
    let attr = |name| element.value().attr(name)?.parse::<f64>().ok();
    BoundingBox {
        x: attr("data-bbox-x")?,
        y: attr("data-bbox-y")?,
        width: attr("data-bbox-width")?,
        height: attr("data-bbox-height")?,
    }
    .validate()
    .ok()
}
fn key(target: &Target) -> (String, [u64; 4]) {
    (
        target.label.clone(),
        [
            target.bbox.x.to_bits(),
            target.bbox.y.to_bits(),
            target.bbox.width.to_bits(),
            target.bbox.height.to_bits(),
        ],
    )
}
fn extract(
    html: &Html,
    attribute: &str,
    secrets: &[String],
    label_only: bool,
) -> Result<Vec<Target>, Error> {
    let selector =
        Selector::parse(&format!("[{attribute}]")).map_err(|_| Error::Invalid("HTML selector"))?;
    let mut seen = BTreeSet::new();
    let mut targets = vec![];
    for element in html.select(&selector) {
        let label = element.value().attr(attribute).unwrap().trim();
        if label.is_empty() {
            continue;
        }
        if label.len() > 1024 {
            return Err(Error::Limit("HTML label"));
        }
        if let Some(bbox) = bounding_box(element) {
            let target = Target {
                label: scrub_string(label, secrets),
                bbox,
            };
            let identity = if label_only {
                (target.label.clone(), [0; 4])
            } else {
                key(&target)
            };
            if seen.insert(identity) {
                targets.push(target);
            }
            if targets.len() > 1024 {
                return Err(Error::Limit("HTML targets"));
            }
        }
    }
    Ok(targets)
}
fn extract_text(html: &Html, secrets: &[String]) -> Result<Vec<Target>, Error> {
    let selector = Selector::parse("*").unwrap();
    let mut seen = BTreeSet::new();
    let mut targets = vec![];
    for element in html.select(&selector) {
        if element
            .children()
            .any(|node| ElementRef::wrap(node).is_some())
        {
            continue;
        }
        let text = element
            .text()
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !(3..=80).contains(&text.chars().count()) {
            continue;
        }
        // Upstream stops at the nearest bbox-bearing ancestor even when its hit test fails.
        let bbox_owner = std::iter::once(element)
            .chain(element.ancestors().take(64).filter_map(ElementRef::wrap))
            .find(|element| {
                [
                    "data-bbox-x",
                    "data-bbox-y",
                    "data-bbox-width",
                    "data-bbox-height",
                ]
                .iter()
                .all(|name| element.value().attr(name).is_some())
            });
        if let Some(bbox) = bbox_owner.and_then(bounding_box) {
            let target = Target {
                label: scrub_string(&text, secrets),
                bbox,
            };
            if seen.insert(key(&target)) {
                targets.push(target);
            }
            if targets.len() > 1024 {
                return Err(Error::Limit("HTML text targets"));
            }
        }
    }
    Ok(targets)
}
fn round4(value: f64) -> f64 {
    (value * 10_000.0).round_ties_even() / 10_000.0
}
#[derive(Debug, Serialize)]
pub struct GuiR1Row {
    pub image_sha256: String,
    pub coordinate_space: &'static str,
    pub gt_bbox: [f64; 4],
    pub instruction: String,
    pub gt_action: &'static str,
    pub gt_input_text: &'static str,
    pub history: &'static str,
    pub task_type: &'static str,
    pub os_type: String,
    pub resolution: String,
}
/// CUA ARIA preference, instruction fallback, center-hit filtering and deduplication.
pub fn gui_r1(input: &ProcessorInput) -> Result<Vec<GuiR1Row>, Error> {
    input.validate()?;
    let html = Html::parse_document(&input.annotated_html);
    if html.tree.nodes().count() > 10_000 {
        return Err(Error::Limit("HTML node count"));
    }
    let mut items = extract(&html, "aria-label", &input.secret_values, false)?;
    if items.is_empty() {
        items = extract(&html, "data-instruction", &input.secret_values, false)?;
    }
    let norm = |value: f64, denom: u32| round4((value / f64::from(denom)).clamp(0.0, 1.0));
    Ok(items
        .into_iter()
        .map(|item| GuiR1Row {
            image_sha256: input.image_sha256.clone(),
            coordinate_space: "normalized_0_1",
            gt_bbox: [
                norm(item.bbox.x, input.width),
                norm(item.bbox.y, input.height),
                norm(item.bbox.x + item.bbox.width, input.width),
                norm(item.bbox.y + item.bbox.height, input.height),
            ],
            instruction: format!("click the UI element {}", item.label),
            gt_action: "click",
            gt_input_text: "no input text",
            history: "None",
            task_type: "low",
            os_type: scrub_string(&input.os_type, &input.secret_values),
            resolution: format!("{}x{}", input.width, input.height),
        })
        .collect())
}
struct RgbImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}
impl RgbImage {
    fn pixel(&self, x: u32, y: u32) -> &[u8] {
        let offset = (y as usize * self.width as usize + x as usize) * 3;
        &self.pixels[offset..offset + 3]
    }
}
fn shrink(image: &RgbImage, bbox: BoundingBox) -> Option<BoundingBox> {
    let mut x = bbox
        .x
        .round_ties_even()
        .clamp(0.0, f64::from(image.width - 1)) as u32;
    let mut y = bbox
        .y
        .round_ties_even()
        .clamp(0.0, f64::from(image.height - 1)) as u32;
    let mut width = bbox
        .width
        .round_ties_even()
        .max(1.0)
        .min(f64::from(image.width - x)) as u32;
    let mut height = bbox
        .height
        .round_ties_even()
        .max(1.0)
        .min(f64::from(image.height - y)) as u32;
    if width < 2 || height < 2 {
        return None;
    }
    while width > 2 && (y..y + height).all(|row| image.pixel(x, row) == image.pixel(x + 1, row)) {
        x += 1;
        width -= 1;
    }
    while height > 2 && (x..x + width).all(|col| image.pixel(col, y) == image.pixel(col, y + 1)) {
        y += 1;
        height -= 1;
    }
    while width > 2
        && (y..y + height)
            .all(|row| image.pixel(x + width - 1, row) == image.pixel(x + width - 2, row))
    {
        width -= 1;
    }
    while height > 2
        && (x..x + width)
            .all(|col| image.pixel(col, y + height - 1) == image.pixel(col, y + height - 2))
    {
        height -= 1;
    }
    Some(BoundingBox {
        x: f64::from(x),
        y: f64::from(y),
        width: f64::from(width),
        height: f64::from(height),
    })
}
fn pool(
    items: Vec<Target>,
    input: &ProcessorInput,
    image: Option<&RgbImage>,
    do_shrink: bool,
) -> Vec<(String, [f64; 2])> {
    items
        .into_iter()
        .filter_map(|item| {
            let bbox = item.bbox;
            if bbox.x <= 0.0
                || bbox.y <= 0.0
                || bbox.width >= f64::from(input.width)
                || bbox.height >= f64::from(input.height)
            {
                return None;
            }
            let bbox = if do_shrink {
                shrink(image?, bbox)?
            } else {
                bbox
            };
            let center = bbox.center();
            let point = [
                round4((center[0] / f64::from(input.width)).clamp(0.0, 1.0)),
                round4((center[1] / f64::from(input.height)).clamp(0.0, 1.0)),
            ];
            if point.iter().any(|v| *v <= 0.0 || *v >= 1.0) {
                None
            } else {
                Some((item.label, point))
            }
        })
        .collect()
}
#[derive(Debug, Serialize)]
pub struct AugmentedPair {
    pub user: String,
    pub assistant: String,
    pub action: String,
    pub point: [f64; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_point: Option<[f64; 2]>,
}
#[derive(Debug, Serialize)]
pub struct AguvisRow {
    pub image_sha256: String,
    pub coordinate_space: &'static str,
    pub source: &'static str,
    pub seed: u64,
    pub rng_version: &'static str,
    pub texts: Vec<AugmentedPair>,
    pub text_pool_available: bool,
    pub predefined_actions_supported: bool,
}
/// Versioned SplitMix64 RNG. Deterministic Rust output; Python random choices differ.
struct Rng(u64);
impl Rng {
    fn index(&mut self, size: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^= z >> 31;
        (z % size as u64) as usize
    }
}
pub fn aguvis(input: &ProcessorInput, seed: u64, n: usize) -> Result<AguvisRow, Error> {
    input.validate()?;
    if n > 100 {
        return Err(Error::Limit("augmentation count"));
    }
    let html = Html::parse_document(&input.annotated_html);
    if html.tree.nodes().count() > 10_000 {
        return Err(Error::Limit("HTML node count"));
    }
    let image = input.rgb()?;
    let text_targets = extract_text(&html, &input.secret_values)?;
    if image.is_some()
        && text_targets
            .iter()
            .map(|t| {
                t.bbox.width.min(f64::from(input.width))
                    * t.bbox.height.min(f64::from(input.height))
            })
            .sum::<f64>()
            > 16_000_000.0
    {
        return Err(Error::Limit("border-shrink work budget"));
    }
    let pools = [
        pool(
            extract(&html, "data-instruction", &input.secret_values, true)?,
            input,
            None,
            false,
        ),
        pool(
            extract(&html, "aria-label", &input.secret_values, false)?,
            input,
            None,
            false,
        ),
        pool(text_targets, input, image.as_ref(), true),
    ];
    let action_pool = predefined_actions(&html, input)?;
    let mut rng = Rng(seed);
    let mut pairs = vec![];
    let mut seen = BTreeSet::new();
    let actions = ["double_click", "right_click", "click", "move_mouse"];
    let templates = [
        ["Double-click to", "Right-click to", "Click to", "Move to"],
        [
            "Double-click the",
            "Right-click the",
            "Click the",
            "Move to the",
        ],
        [
            "Double-click the text",
            "Right-click the text",
            "Click the text",
            "Move to the text",
        ],
    ];
    for _ in 0..(n * 10).max(50) {
        if pairs.len() >= n {
            break;
        }
        let pool_index = rng.index(if action_pool.is_empty() { 3 } else { 4 });
        if pool_index == 3 {
            let candidate = &action_pool[rng.index(action_pool.len())];
            if seen.insert((candidate.user.clone(), candidate.assistant.clone())) {
                pairs.push(AugmentedPair {
                    user: candidate.user.clone(),
                    assistant: candidate.assistant.clone(),
                    action: candidate.action.clone(),
                    point: candidate.point,
                    to_point: candidate.to_point,
                });
            }
            continue;
        }
        let pool = &pools[pool_index];
        if pool.is_empty() {
            continue;
        }
        let (label, point) = &pool[rng.index(pool.len())];
        let verb = rng.index(4);
        let user = format!("{} '{}'", templates[pool_index][verb], label);
        let assistant = format!("{}(x={}, y={})", actions[verb], point[0], point[1]);
        if seen.insert((user.clone(), assistant.clone())) {
            pairs.push(AugmentedPair {
                user,
                assistant,
                action: actions[verb].into(),
                point: *point,
                to_point: None,
            });
        }
    }
    Ok(AguvisRow {
        image_sha256: input.image_sha256.clone(),
        coordinate_space: "normalized_0_1",
        source: "cua-bench",
        seed,
        rng_version: "splitmix64-v1",
        texts: pairs,
        text_pool_available: image.is_some(),
        predefined_actions_supported: true,
    })
}

fn predefined_actions(html: &Html, input: &ProcessorInput) -> Result<Vec<AugmentedPair>, Error> {
    let selector = Selector::parse("[data-actions]").unwrap();
    let mut pool = vec![];
    let mut seen = BTreeSet::new();
    for element in html.select(&selector) {
        let raw = element.value().attr("data-actions").unwrap();
        if raw.len() > 64 * 1024 {
            return Err(Error::Limit("predefined actions bytes"));
        }
        let values: Vec<serde_json::Value> =
            serde_json::from_str(raw).map_err(|_| Error::Invalid("predefined actions JSON"))?;
        if values.len() > 1024 {
            return Err(Error::Limit("predefined actions count"));
        }
        let space = input.action_coordinate_space.ok_or(Error::Invalid(
            "predefined action coordinate space required",
        ))?;
        for value in values {
            let user = value
                .get("user")
                .and_then(serde_json::Value::as_str)
                .ok_or(Error::Invalid("predefined user"))?;
            let assistant = value
                .get("assistant")
                .and_then(serde_json::Value::as_str)
                .ok_or(Error::Invalid("predefined assistant"))?;
            if user.len() > 1024 || assistant.len() > 1024 {
                return Err(Error::Limit("predefined action text"));
            }
            let compact: String = assistant.chars().filter(|ch| !ch.is_whitespace()).collect();
            let normalize = |point: [f64; 2]| -> Result<[f64; 2], Error> {
                let denominators = if space == crate::benchmark::CoordinateSpace::ImagePixels {
                    [f64::from(input.width), f64::from(input.height)]
                } else {
                    [1.0, 1.0]
                };
                let normalized = [
                    round4(point[0] / denominators[0]),
                    round4(point[1] / denominators[1]),
                ];
                if normalized
                    .iter()
                    .any(|v| !v.is_finite() || *v <= 0.0 || *v >= 1.0)
                {
                    return Err(Error::Invalid("predefined coordinates outside image"));
                }
                Ok(normalized)
            };
            let pair = |raw: &str| -> Result<[f64; 2], Error> {
                let (x, y) = raw
                    .split_once(',')
                    .ok_or(Error::Invalid("predefined coordinate pair"))?;
                Ok([
                    x.parse().map_err(|_| Error::Invalid("predefined x"))?,
                    y.parse().map_err(|_| Error::Invalid("predefined y"))?,
                ])
            };
            let (name, arguments) = compact
                .split_once('(')
                .ok_or(Error::Invalid("predefined action syntax"))?;
            let arguments = arguments
                .strip_suffix(')')
                .ok_or(Error::Invalid("predefined action suffix"))?;
            let (point, to_point) = if name == "drag" {
                let args = arguments
                    .strip_prefix("from_coord=[")
                    .and_then(|v| v.strip_suffix(']'))
                    .ok_or(Error::Invalid("predefined drag syntax"))?;
                let (from, to) = args
                    .split_once("],to_coord=[")
                    .ok_or(Error::Invalid("predefined drag endpoints"))?;
                (normalize(pair(from)?)?, Some(normalize(pair(to)?)?))
            } else if matches!(
                name,
                "click" | "double_click" | "right_click" | "middle_click" | "move_mouse"
            ) {
                let args = arguments
                    .strip_prefix("x=")
                    .ok_or(Error::Invalid("predefined point syntax"))?;
                let (x, y) = args
                    .split_once(",y=")
                    .ok_or(Error::Invalid("predefined point syntax"))?;
                (normalize(pair(&format!("{x},{y}"))?)?, None)
            } else {
                return Err(Error::Invalid("predefined action unsupported"));
            };
            let assistant = if let Some(to) = to_point {
                format!(
                    "drag(from_coord=[{},{}], to_coord=[{},{}])",
                    point[0], point[1], to[0], to[1]
                )
            } else {
                format!("{name}(x={}, y={})", point[0], point[1])
            };
            let user = scrub_string(user, &input.secret_values);
            if seen.insert((user.clone(), assistant.clone())) {
                pool.push(AugmentedPair {
                    user,
                    assistant,
                    action: name.into(),
                    point,
                    to_point,
                });
            }
            if pool.len() > 1024 {
                return Err(Error::Limit("predefined action pool"));
            }
        }
    }
    Ok(pool)
}

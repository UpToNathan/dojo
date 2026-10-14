//! Isolated dataset episodes and CUA grounding metrics. This never controls a desktop.
use crate::{Effect, Error, EventKind, Recorder, RecordingPolicy, Trace, TraceEvent};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinateSpace {
    ImagePixels,
    Normalized01,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    Box { xyxy: [f64; 4] },
    Polygon { points: Vec<[f64; 2]> },
    Infeasible,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Answer {
    Click { x: f64, y: f64 },
    RightClick { x: f64, y: f64 },
    DoubleClick { x: f64, y: f64 },
    MiddleClick { x: f64, y: f64 },
    Move { x: f64, y: f64 },
    ReportInfeasible,
}
impl Answer {
    fn point(&self) -> Option<[f64; 2]> {
        match *self {
            Self::Click { x, y }
            | Self::RightClick { x, y }
            | Self::DoubleClick { x, y }
            | Self::MiddleClick { x, y } => Some([x, y]),
            Self::Move { .. } | Self::ReportInfeasible => None,
        }
    }
    fn coordinates(&self) -> Option<[f64; 2]> {
        if let Self::Move { x, y } = *self {
            Some([x, y])
        } else {
            self.point()
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetTask {
    pub task_id: String,
    pub image_sha256: String,
    pub width: u32,
    pub height: u32,
    pub coordinate_space: CoordinateSpace,
    pub target: Target,
    /// None means missing model evidence. An empty answer is an observed no-click answer.
    pub answers: Option<Vec<Answer>>,
    #[serde(default)]
    pub cancelled_after: Option<usize>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EpisodeLimits {
    pub max_steps: usize,
    pub max_tasks: usize,
}
impl Default for EpisodeLimits {
    fn default() -> Self {
        Self {
            max_steps: 100,
            max_tasks: 100,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Phase {
    pub name: &'static str,
    pub completed: bool,
}
#[derive(Debug, Serialize)]
pub struct EpisodeResult {
    pub task_id: String,
    pub outcome: &'static str,
    pub reward: Option<f64>,
    pub completed_steps: usize,
    pub phases: Vec<Phase>,
    pub trace: Trace,
    pub execution_target: &'static str,
}
#[derive(Debug, Serialize)]
pub struct EpisodeReport {
    pub schema_version: &'static str,
    pub evaluator_version: &'static str,
    pub results: Vec<EpisodeResult>,
    pub completed: usize,
    pub failed: usize,
    pub unknown: usize,
    pub avg_reward: Option<f64>,
}
fn valid_point(point: [f64; 2], space: CoordinateSpace, width: u32, height: u32) -> bool {
    let bounds = if space == CoordinateSpace::Normalized01 {
        [1.0, 1.0]
    } else {
        [f64::from(width), f64::from(height)]
    };
    point.into_iter().zip(bounds).all(|(coordinate, bound)| {
        coordinate.is_finite() && coordinate >= 0.0 && coordinate <= bound
    })
}
impl DatasetTask {
    fn validate(&self) -> Result<(), Error> {
        if self.task_id.is_empty()
            || self.task_id.len() > 128
            || self.task_id.chars().any(char::is_control)
        {
            return Err(Error::Invalid("dataset task ID"));
        }
        if self.width == 0
            || self.height == 0
            || u64::from(self.width) * u64::from(self.height) > 33_554_432
        {
            return Err(Error::Limit("dataset image dimensions"));
        }
        if self.image_sha256.len() != 64
            || !self.image_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::Invalid("dataset image digest"));
        }
        let point_ok = |point| valid_point(point, self.coordinate_space, self.width, self.height);
        match &self.target {
            Target::Box { xyxy } => {
                if !point_ok([xyxy[0], xyxy[1]]) || !point_ok([xyxy[2], xyxy[3]]) {
                    return Err(Error::Invalid("dataset target bbox"));
                }
            }
            Target::Polygon { points } => {
                if !(3..=256).contains(&points.len())
                    || points.iter().any(|point| !point_ok(*point))
                {
                    return Err(Error::Invalid("dataset target polygon"));
                }
            }
            Target::Infeasible => {}
        }
        if self.answers.as_ref().is_some_and(|answers| {
            answers.len() > 10_000
                || answers
                    .iter()
                    .any(|a| a.coordinates().is_some_and(|point| !point_ok(point)))
        }) {
            return Err(Error::Invalid("dataset answers"));
        }
        Ok(())
    }
}
/// Upstream box metric accepts reversed endpoints and includes the boundary.
pub fn in_xyxy(point: [f64; 2], xyxy: [f64; 4]) -> bool {
    [point[0], point[1], xyxy[0], xyxy[1], xyxy[2], xyxy[3]]
        .iter()
        .all(|v| v.is_finite())
        && xyxy[0].min(xyxy[2]) <= point[0]
        && point[0] <= xyxy[0].max(xyxy[2])
        && xyxy[1].min(xyxy[3]) <= point[1]
        && point[1] <= xyxy[1].max(xyxy[3])
}
/// Upstream even-odd rule, with edge points inside and a 1e-9 cross-product tolerance.
pub fn in_polygon(point: [f64; 2], points: &[[f64; 2]]) -> bool {
    if points.len() < 3
        || points.len() > 256
        || point
            .iter()
            .chain(points.iter().flatten())
            .any(|v| !v.is_finite())
    {
        return false;
    }
    let [x, y] = point;
    let mut inside = false;
    for i in 0..points.len() {
        let [x0, y0] = points[i];
        let [x1, y1] = points[(i + 1) % points.len()];
        if x0.min(x1) <= x
            && x <= x0.max(x1)
            && y0.min(y1) <= y
            && y <= y0.max(y1)
            && ((x1 - x0) * (y - y0) - (y1 - y0) * (x - x0)).abs() < 1e-9
        {
            return true;
        }
        if (y0 > y) != (y1 > y) && x < (x1 - x0) * (y - y0) / (y1 - y0) + x0 {
            inside = !inside;
        }
    }
    inside
}
/// Exactly-one-click ScreenSpot protocol. Move answers do not count as clicks.
pub fn grounding_reward(target: &Target, answers: &[Answer]) -> f64 {
    let points: Vec<_> = answers.iter().filter_map(Answer::point).collect();
    let pass = match target {
        Target::Box { xyxy } => points.len() == 1 && in_xyxy(points[0], *xyxy),
        Target::Polygon { points: polygon } => points.len() == 1 && in_polygon(points[0], polygon),
        Target::Infeasible => {
            points.is_empty()
                && answers
                    .iter()
                    .any(|a| matches!(a, Answer::ReportInfeasible))
        }
    };
    if pass { 1.0 } else { 0.0 }
}
/// Execute precomputed answers only in an in-memory dataset session. No retry or scheduler.
/// Setup/evaluation/cleanup are finite pure operations, and answer recording has a step budget.
pub fn run_episode(
    task: &DatasetTask,
    limits: &EpisodeLimits,
    policy: RecordingPolicy,
) -> Result<EpisodeResult, Error> {
    task.validate()?;
    if limits.max_steps > 1000 || limits.max_tasks > 1000 {
        return Err(Error::Limit("episode limits"));
    }
    let task_id = crate::scrub_string(&task.task_id, &policy.secret_values);
    let mut recorder = Recorder::new(&task.task_id, policy)?;
    let mut time_ms = 0;
    let observation = |time_ms, kind, data| TraceEvent {
        time_ms,
        kind,
        pi_intent_id: None,
        call_id: None,
        route: Some("dataset".into()),
        observation_id: Some(task.image_sha256.clone()),
        effect: None,
        data,
    };
    recorder.push(observation(time_ms, EventKind::Observation, json!({"phase": "setup", "width": task.width,
        "height": task.height, "coordinate_space": task.coordinate_space, "image_sha256": task.image_sha256})))?;
    let mut phases = vec![Phase {
        name: "setup",
        completed: true,
    }];
    let mut applied = vec![];
    let mut interrupted = false;
    if let Some(answers) = &task.answers {
        for (index, answer) in answers.iter().enumerate() {
            if index >= limits.max_steps || task.cancelled_after.is_some_and(|limit| index >= limit)
            {
                interrupted = true;
                break;
            }
            // These IDs identify fixture answers only. They are never presented as Pi intents.
            let id = format!("dataset-answer-{index}");
            time_ms += 1;
            let event = TraceEvent {
                time_ms,
                kind: EventKind::Intent,
                pi_intent_id: Some(id.clone()),
                call_id: Some(id),
                route: Some("dataset".into()),
                observation_id: Some(task.image_sha256.clone()),
                effect: None,
                data: json!({"answer": answer, "coordinate_space": task.coordinate_space, "fixture_intent": true}),
            };
            recorder.push(event.clone())?;
            applied.push(answer.clone());
            time_ms += 1;
            let mut receipt = event;
            receipt.time_ms = time_ms;
            receipt.kind = EventKind::Receipt;
            receipt.effect = Some(Effect::Confirmed);
            receipt.data = json!({"recorded_dataset_answer": true, "desktop_effect": false});
            recorder.push(receipt)?;
        }
    }
    phases.push(Phase {
        name: "answer",
        completed: !interrupted && task.answers.is_some(),
    });
    let reward = if interrupted || task.answers.is_none() {
        None
    } else {
        Some(grounding_reward(&task.target, &applied))
    };
    let outcome = match reward {
        Some(1.0) => "success",
        Some(_) => "failure",
        None => "unknown",
    };
    time_ms += 1;
    recorder.push(observation(
        time_ms,
        EventKind::Verification,
        json!({"phase": "evaluate", "reward": reward,
        "outcome": outcome, "interrupted": interrupted}),
    ))?;
    phases.push(Phase {
        name: "evaluate",
        completed: reward.is_some(),
    });
    // No external files, process, seat or held input exists to leak during cleanup.
    time_ms += 1;
    recorder.push(observation(
        time_ms,
        EventKind::RunEnd,
        json!({"phase": "cleanup", "released_dataset_session": true}),
    ))?;
    phases.push(Phase {
        name: "cleanup",
        completed: true,
    });
    Ok(EpisodeResult {
        task_id,
        outcome,
        reward,
        completed_steps: applied.len(),
        phases,
        trace: recorder.finish(),
        execution_target: "isolated_in_memory_dataset",
    })
}
pub fn run_tasks(
    tasks: &[DatasetTask],
    limits: &EpisodeLimits,
    policy: RecordingPolicy,
) -> Result<EpisodeReport, Error> {
    if tasks.len() > limits.max_tasks || tasks.len() > 1000 {
        return Err(Error::Limit("dataset task count"));
    }
    let mut ids = std::collections::BTreeSet::new();
    for task in tasks {
        if !ids.insert(&task.task_id) {
            return Err(Error::Invalid("duplicate dataset task ID"));
        }
        task.validate()?;
    }
    let mut results = Vec::new();
    let mut total_bytes = 0usize;
    for task in tasks {
        let result = run_episode(task, limits, policy.clone())?;
        total_bytes += serde_json::to_vec(&result)?.len();
        if total_bytes > crate::MAX_INPUT_BYTES {
            return Err(Error::Limit("benchmark report bytes"));
        }
        results.push(result);
    }
    let rewards: Vec<_> = results.iter().filter_map(|result| result.reward).collect();
    let avg_reward = if rewards.is_empty() {
        None
    } else {
        Some(rewards.iter().sum::<f64>() / rewards.len() as f64)
    };
    Ok(EpisodeReport {
        schema_version: "dojo.computer.benchmark/1",
        evaluator_version: "cua-grounding@88aff7e",
        completed: results.iter().filter(|r| r.reward.is_some()).count(),
        failed: results.iter().filter(|r| r.reward == Some(0.0)).count(),
        unknown: results.iter().filter(|r| r.reward.is_none()).count(),
        results,
        avg_reward,
    })
}

//! Bringing a machine into a mesh through an invite: `invite` and `requests` and `deny` for the
//! machine that signs, `approve --invite` to let one in, and `join --invite` for the new machine. The
//! messages travel by the configured rendezvous (1Password by default), so nothing is carried by hand
//! and no fingerprint is compared: the carrier is the person's own account, which is already what
//! holds the nodes' keys. The request is still signed by the new machine's own key.

use crate::cli::{Env, FAILED, Failure, NEEDS_HUMAN, UNAVAILABLE, USAGE_ERROR, fail, has, split};
use crate::enroll::{self, JoinRequest};
use crate::enroll_cli::{self, Ctx};
use crate::openssh::SshKeygen;
use crate::rendezvous::{self, Answer, Status};
use crate::rostersync;
use chrono::Utc;
use std::time::{Duration, Instant};

/// How often the new machine looks for an answer.
const POLL: Duration = Duration::from_millis(2000);
/// How long the new machine waits for an invite, and then for approval, unless told otherwise.
const WAIT_SECONDS: u64 = 15 * 60;

fn carrier_error(message: String) -> Failure {
    let code = if message.contains("unlock") || message.contains("needs you") { NEEDS_HUMAN } else { UNAVAILABLE };
    fail(code, message)
}

fn poll_interval(values: &[(String, String)]) -> Duration {
    enroll_cli::value(values, "--poll").and_then(|text| text.parse::<u64>().ok()).map(Duration::from_millis).unwrap_or(POLL)
}

/// `dojo-mesh invite [--minutes N] [--cancel ID] [--json]`: makes an invite, or withdraws one.
pub fn invite(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let ctx = enroll_cli::context(env)?;
    let (words, values) = split(rest, &["--json"], &["--minutes", "--cancel"])?;
    if !words.positional.is_empty() {
        return Err(fail(USAGE_ERROR, "usage: dojo-mesh invite [--minutes N] [--cancel ID] [--json]"));
    }
    let carrier = rendezvous::open(&ctx.config.rendezvous);
    let now = Utc::now();
    let _ = rendezvous::sweep(carrier.as_ref(), now);
    if let Some(id) = enroll_cli::value(&values, "--cancel") {
        carrier.delete(&id).map_err(carrier_error)?;
        println!("withdrawn");
        return Ok(0);
    }
    let minutes = match enroll_cli::value(&values, "--minutes") {
        Some(text) => text.parse::<i64>().ok().filter(|minutes| (1..=60).contains(minutes)).ok_or_else(|| fail(USAGE_ERROR, "--minutes takes 1 to 60"))?,
        None => rendezvous::INVITE_MINUTES,
    };
    let signers = enroll::signers(&std::fs::read_to_string(&ctx.paths.signers).unwrap_or_default());
    let invite = rendezvous::post_invite(carrier.as_ref(), &enroll_cli::node_name(env, &ctx.config), &signers, now, minutes).map_err(carrier_error)?;
    if has(&words, "--json") {
        println!("{}", serde_json::json!({ "id": invite.id, "expires": invite.expires.to_rfc3339() }));
    } else {
        println!("Invite ready until {}. On the new machine, run: dojo-mesh join --invite", invite.expires.with_timezone(&chrono::Local).format("%H:%M"));
    }
    Ok(0)
}

/// `dojo-mesh requests [--json]`: the machines waiting to be let in.
pub fn requests(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let ctx = enroll_cli::context(env)?;
    let (words, _) = split(rest, &["--json"], &[])?;
    let carrier = rendezvous::open(&ctx.config.rendezvous);
    let (waiting, skipped) = rendezvous::pending(carrier.as_ref(), Utc::now()).map_err(carrier_error)?;
    if has(&words, "--json") {
        let list: Vec<serde_json::Value> = waiting
            .iter()
            .map(|p| serde_json::json!({ "invite": p.invite.id, "name": p.request.name, "routes": p.request.routes.iter().map(|r| r.host.clone()).collect::<Vec<_>>(), "expires": p.invite.expires.to_rfc3339() }))
            .collect();
        println!("{}", serde_json::to_string(&list).map_err(|error| fail(FAILED, error.to_string()))?);
    } else if waiting.is_empty() {
        println!("No machine is waiting to join.");
    }
    for pending in &waiting {
        if !has(&words, "--json") {
            let fingerprint = enroll::fingerprint(&pending.request.node_key).unwrap_or_default();
            println!("{}  wants to join  (invite {}, key {fingerprint})", pending.request.name, pending.invite.id);
        }
    }
    if skipped > 0 {
        eprintln!("dojo-mesh: {skipped} request(s) were not taken: unreadable, or not signed by their own key");
    }
    Ok(0)
}

/// `dojo-mesh deny ID [--reason TEXT]`: turns a request down.
pub fn deny(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let ctx = enroll_cli::context(env)?;
    let (words, values) = split(rest, &[], &["--reason"])?;
    let [id] = words.positional.as_slice() else { return Err(fail(USAGE_ERROR, "usage: dojo-mesh deny INVITE_ID [--reason TEXT]")) };
    let carrier = rendezvous::open(&ctx.config.rendezvous);
    rendezvous::deny(carrier.as_ref(), id, &enroll_cli::value(&values, "--reason").unwrap_or_else(|| "the request was turned down".into())).map_err(carrier_error)?;
    println!("turned down");
    Ok(0)
}

/// `dojo-mesh approve --invite ID`: signs the waiting machine in and sends it the roster.
pub(crate) fn approve_invite(env: &Env, ctx: &Ctx, id: &str, days: i64, replace: bool) -> Result<u8, Failure> {
    let carrier = rendezvous::open(&ctx.config.rendezvous);
    let (waiting, _) = rendezvous::pending(carrier.as_ref(), Utc::now()).map_err(carrier_error)?;
    let pending = waiting.into_iter().find(|pending| pending.invite.id == id).ok_or_else(|| fail(FAILED, "no request is waiting under that invite (it may have expired, or been answered)"))?;
    enroll_cli::approve_core(env, ctx, &pending.request, days, replace)?;
    let bundle = rostersync::export(&ctx.paths.dir).map_err(|message| fail(FAILED, message))?;
    rendezvous::post_response(carrier.as_ref(), id, &bundle).map_err(carrier_error)?;
    println!("{} can finish joining now", pending.request.name);
    Ok(0)
}

/// The new machine: finds an open invite, leaves its signed request, waits for the answer and installs it.
pub(crate) fn join_through_invite(env: &Env, ctx: &Ctx, request: &JoinRequest, values: &[(String, String)]) -> Result<u8, Failure> {
    let carrier = rendezvous::open(&ctx.config.rendezvous);
    let wait = Duration::from_secs(enroll_cli::value(values, "--wait").and_then(|text| text.parse().ok()).unwrap_or(WAIT_SECONDS));
    let interval = poll_interval(values);
    let deadline = Instant::now() + wait;

    // An invite: the one named, or the only open one, waited for if there is none yet.
    let chosen = loop {
        let open: Vec<_> = rendezvous::invites(carrier.as_ref(), Utc::now()).map_err(carrier_error)?.into_iter().filter(|invite| invite.status == Status::Open).collect();
        let wanted = enroll_cli::value(values, "--invite-id");
        let candidates: Vec<_> = open.into_iter().filter(|invite| wanted.as_deref().is_none_or(|id| invite.id == id)).collect();
        match candidates.as_slice() {
            [one] => break one.clone(),
            [] if Instant::now() < deadline => {
                eprintln!("dojo-mesh: no invite yet; on the machine to join, make one (the mesh panel's Add machine, or dojo-mesh invite)");
                std::thread::sleep(interval);
            }
            [] => return Err(fail(FAILED, "no invite was waiting")),
            many => {
                let names: Vec<_> = many.iter().map(|invite| format!("{} ({})", invite.operator, invite.id)).collect();
                return Err(fail(USAGE_ERROR, format!("more than one invite is open: {}; choose one with --invite-id", names.join(", "))));
            }
        }
    };
    rendezvous::post_request(carrier.as_ref(), &chosen.id, request, Utc::now()).map_err(carrier_error)?;
    eprintln!("dojo-mesh: asked {} to let {} in; waiting for approval", chosen.operator, request.name);

    loop {
        match rendezvous::answer(carrier.as_ref(), &chosen.id, Utc::now()).map_err(carrier_error)? {
            Answer::Approved { bundle, signers } => {
                for signer in &signers {
                    enroll_cli::trust_key(ctx, signer)?;
                }
                let verifier = SshKeygen { signers: ctx.paths.signers.clone() };
                let merged = rostersync::merge(&ctx.paths.dir, &bundle, &verifier).map_err(|message| fail(FAILED, message))?;
                println!("took the roster: {} added, {} updated", merged.added.len(), merged.updated.len());
                if let Err(failure) = crate::cli::authorize(env, &ctx.config) {
                    eprintln!("dojo-mesh: could not update authorized_keys: {}", failure.message);
                }
                let _ = carrier.delete(&chosen.id);
                println!("{} joined {}'s mesh", request.name, chosen.operator);
                return Ok(0);
            }
            Answer::Denied(reason) => return Err(fail(FAILED, format!("{} turned the request down: {reason}", chosen.operator))),
            Answer::Gone => return Err(fail(FAILED, "the invite ran out or was withdrawn before it was answered")),
            Answer::Waiting if Instant::now() >= deadline => return Err(fail(FAILED, "no answer in time; the invite is still open, run this again to keep waiting")),
            Answer::Waiting => std::thread::sleep(interval),
        }
    }
}


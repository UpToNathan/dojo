use dojo_external_admission::*;
use std::num::NonZeroU64;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--sockets") {
        #[cfg(feature = "socket-fixtures")]
        {
            println!("{}", sockets::prove_loopback_websocket()?);
            println!("{}", sockets::prove_unix_dbus()?);
            for report in sockets::prove_unix_pressure()? {
                println!("{report}");
            }
        }
        #[cfg(not(feature = "socket-fixtures"))]
        return Err("socket fixtures feature is disabled".into());
        return Ok(());
    }

    for route in [Route::Cdp, Route::Atspi] {
        let target = fixture_target();
        let mut actor = Actor::new(target.compositor_epoch);
        actor.install_target(target)?;
        let mutation = match route {
            Route::Cdp => Mutation::CdpNavigate {
                url: "https://fixture.invalid/".into(),
            },
            Route::Atspi => Mutation::AtspiSetValue {
                object: "/fixture/field".into(),
                value: "fixture".into(),
            },
        };
        let grant = actor.grant(target, mutation, 100)?;
        let stamp = actor.observe(target, NonZeroU64::new(1).unwrap())?;
        let handle = actor.prepare(stamp, grant, 90)?;
        let mut transport = match route {
            Route::Cdp => FixtureTransport::cdp(),
            Route::Atspi => FixtureTransport::dbus(),
        };
        println!("{route:?}\tprepared\t{:?}", actor.inspect(&handle)?);
        println!(
            "{route:?}\tcommit\t{:?}",
            actor.try_dispatch(
                &handle,
                Requirement::NoTransportCommitAfterTakeover,
                &mut transport
            )?
        );
        actor.human_takeover(target.client);
        println!("{route:?}\thuman takeover\t{:?}", actor.inspect(&handle)?);
        assert!(transport.dispatch(0));
        assert!(transport.apply_effect(0));
        assert!(transport.prepare_receipt(0));
        println!(
            "{route:?}\tlate receipt\t{:?}",
            transport.acknowledge(0, &mut actor)?
        );
        println!(
            "{route:?}\treplay\t{:?}",
            actor.try_dispatch(
                &handle,
                Requirement::NoTransportCommitAfterTakeover,
                &mut transport
            )
        );
    }
    Ok(())
}

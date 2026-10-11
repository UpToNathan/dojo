"""Check frame-driven pointer movement against the raw Wayland recorder."""
import time
from run import Peer, events, screenshot, wait_for


def verify_motion(agent, query, output, x, y, seat_id, log_path):
    def move(px, py):
        return agent.request({"AgentInput": {"Move": {"output": output, "x": px, "y": py}}})

    def recorded_motion(start):
        return [e for e in events(log_path)[start:]
                if e.get("seat") == seat_id and e.get("event") == "motion"]

    start_x, start_y = x - 130, y - 80
    move(start_x, start_y)
    agent.agent("Wait")
    time.sleep(0.05)
    initial = [e for e in events(log_path) if e.get("seat") == seat_id
               and e.get("event") in ("motion", "pointer_enter")][-1]
    screenshot(query, log_path + ".motion-start.png")
    start = len(events(log_path))
    assert move(x, y) == "AgentInputMoving"
    time.sleep(0.07)
    screenshot(query, log_path + ".motion-mid.png")
    agent.agent("Wait")
    screenshot(query, log_path + ".motion-end.png")
    samples = wait_for(lambda: recorded_motion(start) if len(recorded_motion(start)) >= 4 else None,
                       "at least four intermediate motion events")
    assert abs(samples[-1]["x"] - initial["x"] - 130) < 0.02
    assert abs(samples[-1]["y"] - initial["y"] - 80) < 0.02
    previous = initial
    for sample in samples:
        assert sample["x"] >= previous["x"] - 0.02
        assert sample["y"] >= previous["y"] - 0.02
        previous = sample

    # Raw clients can click immediately. Niri must defer the press until arrival.
    start = len(events(log_path))
    move(start_x, start_y)
    before = time.monotonic()
    agent.agent({"Button": {"button": 272, "state": "Pressed"}})
    elapsed = time.monotonic() - before
    assert elapsed > 0.12, f"Click skipped movement: {elapsed}s"
    agent.agent({"Button": {"button": 272, "state": "Released"}})
    wait_for(lambda: any(e.get("seat") == seat_id and e.get("event") == "button"
                        for e in events(log_path)[start:]), "deferred click")
    delivered = [e for e in events(log_path)[start:] if e.get("seat") == seat_id]
    click_index = next(i for i, e in enumerate(delivered) if e.get("event") == "button")
    arrived = [e for e in delivered[:click_index] if e.get("event") == "motion"][-1]
    assert abs(arrived["x"] - initial["x"]) < 0.02
    assert abs(arrived["y"] - initial["y"]) < 0.02

    # A second raw Move can redirect the first before it finishes.
    move(x, y)
    time.sleep(0.08)
    assert move(start_x, start_y) == "AgentInputMoving"
    agent.agent("Wait")
    time.sleep(0.05)
    last = recorded_motion(0)[-1]
    assert abs(last["x"] - initial["x"]) < 0.02
    assert abs(last["y"] - initial["y"]) < 0.02

    move(x, y)
    agent.agent("Reset")
    time.sleep(0.08)
    stopped = len(recorded_motion(0))
    screenshot(query, log_path + ".motion-reset.png")
    time.sleep(0.6)
    assert len(recorded_motion(0)) == stopped, "Reset left motion running"
    screenshot(query, log_path + ".motion-reset-later.png")
    disconnected = Peer(agent.socket.getpeername())
    try:
        created = disconnected.agent({"Create": {"name": "motion-disconnect"}})
        name = created["AgentInputCreated"]["seat_name"]
        seat = wait_for(lambda: next((e["seat"] for e in events(log_path)
                        if e.get("event") == "seat_name" and e.get("name") == name), None),
                        "temporary seat announcement")
        assert disconnected.request({"AgentInput": {"Move": {"output": output, "x": x, "y": y}}}) == "AgentInputMoving"
    finally:
        disconnected.close()
    wait_for(lambda: any(e.get("event") == "seat_removed" and e.get("seat") == seat
                        for e in events(log_path)), "disconnect during movement")
    time.sleep(0.08)
    removed = len([e for e in events(log_path) if e.get("seat") == seat and e.get("event") == "motion"])
    time.sleep(0.6)
    assert len([e for e in events(log_path) if e.get("seat") == seat and e.get("event") == "motion"]) == removed
    print(f"Motion verified: {len(samples)} intermediate events; click waited {elapsed:.3f}s; retarget/reset/disconnect passed")

//! What the token space refuses.
//!
//! Tokens are drawn from one counter shared by every widget on the desktop, so
//! nothing stops a widget guessing its neighbour's — except the owner check.
//! Most of what follows is about that, and about the limits that stop one
//! widget's bug becoming the machine's problem.

use super::*;

const ALICE: WidgetId = WidgetId(1);
const BOB: WidgetId = WidgetId(2);

fn tasks() -> Arc<Tasks> {
    Arc::new(Tasks::default())
}

/// Work that finishes immediately with a known answer.
fn says(what: &str) -> Work {
    let what = what.to_string();
    Box::new(move || {
        TaskResult::Command(CommandOutput {
            status: 0,
            stdout: what.into_bytes(),
            stderr: Vec::new(),
        })
    })
}

/// Waits for `token` to be reported finished, or gives up.
fn settle(tasks: &Tasks, expected: usize) -> Vec<(WidgetId, u32)> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut collected = Vec::new();
    while collected.len() < expected && std::time::Instant::now() < deadline {
        collected.extend(tasks.finished());
        if collected.len() < expected {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    collected
}

#[test]
fn a_request_returns_a_token_immediately_and_the_answer_arrives_later() {
    // The invariant the whole module exists for: starting work does not wait
    // for it. If `start` ever blocked, this would take a second.
    let tasks = tasks();
    let began = std::time::Instant::now();
    let token = tasks
        .start(
            ALICE,
            Box::new(|| {
                std::thread::sleep(Duration::from_millis(200));
                TaskResult::Failed("slept".into())
            }),
        )
        .expect("started");
    assert!(
        began.elapsed() < Duration::from_millis(50),
        "starting a task took {:?}, which means it waited for it",
        began.elapsed()
    );
    assert_eq!(tasks.take(ALICE, token), Err(TakeError::NotReady));

    assert_eq!(settle(&tasks, 1), vec![(ALICE, token)]);
    assert_eq!(
        tasks.take(ALICE, token),
        Ok(TaskResult::Failed("slept".into()))
    );
}

#[test]
fn a_widget_cannot_collect_its_neighbours_result() {
    // The reason a token carries an owner. Both widgets draw from one counter,
    // so Bob can simply try Alice's number — and the answer must not be her
    // command's output.
    let tasks = tasks();
    let hers = tasks.start(ALICE, says("her secret")).expect("started");
    settle(&tasks, 1);

    assert_eq!(tasks.take(BOB, hers), Err(TakeError::NotYours));
    // And the attempt did not consume it.
    assert!(matches!(
        tasks.take(ALICE, hers),
        Ok(TaskResult::Command(_))
    ));
}

#[test]
fn a_token_that_never_existed_reads_the_same_as_one_that_is_not_yours() {
    // Deliberate: telling a widget "that token exists but is not yours" tells
    // it how many requests its neighbours have made.
    let tasks = tasks();
    assert_eq!(tasks.take(BOB, 9999), Err(TakeError::Gone));
}

#[test]
fn a_token_is_a_one_shot_receipt() {
    let tasks = tasks();
    let token = tasks.start(ALICE, says("once")).expect("started");
    settle(&tasks, 1);
    assert!(tasks.take(ALICE, token).is_ok());
    assert_eq!(tasks.take(ALICE, token), Err(TakeError::Gone));
}

#[test]
fn a_widget_that_never_collects_is_refused_rather_than_accumulating() {
    // A widget starting a request on every view and collecting none would
    // otherwise grow the desktop's memory until somebody noticed. Refusing is
    // what makes the bug visible to its author.
    let tasks = tasks();
    for _ in 0..MAX_IN_FLIGHT {
        // One at a time: the desktop-wide budget is smaller than the
        // per-widget one, so these have to finish to make room.
        let token = tasks.start(ALICE, says("x")).expect("started");
        settle(&tasks, 1);
        // Left uncollected on purpose.
        let _ = token;
    }
    assert_eq!(tasks.in_flight(ALICE), MAX_IN_FLIGHT);
    assert_eq!(
        tasks.start(ALICE, says("one too many")),
        Err(StartError::TooManyForWidget)
    );
    // A neighbour is unaffected: the limit is per widget, or one misbehaving
    // widget would take the desktop's whole budget.
    assert!(tasks.start(BOB, says("fine")).is_ok());
}

#[test]
fn the_desktop_will_not_run_unboundedly_many_at_once() {
    // A widget cannot spawn a thread, and this is why: without the budget,
    // eight widgets each allowed sixteen in flight is a hundred and
    // twenty-eight processes.
    let tasks = tasks();
    let gate = Arc::new(std::sync::Barrier::new(MAX_CONCURRENT + 1));
    for _ in 0..MAX_CONCURRENT {
        let gate = Arc::clone(&gate);
        tasks
            .start(
                ALICE,
                Box::new(move || {
                    gate.wait();
                    TaskResult::Failed("held".into())
                }),
            )
            .expect("within the budget");
    }
    assert_eq!(
        tasks.start(BOB, says("no room")),
        Err(StartError::DesktopBusy),
        "the budget is the desktop's, not one widget's"
    );
    // Let them go, and the budget comes back.
    gate.wait();
    settle(&tasks, MAX_CONCURRENT);
    assert!(tasks.start(BOB, says("room now")).is_ok());
}

#[test]
fn finishing_wakes_the_loop_once_and_the_flag_clears() {
    // The desktop blocks on its window system's socket with no timer when
    // nothing is scheduled. Without this flag a task that finished during that
    // block would sit there until something else happened.
    let tasks = tasks();
    assert!(!tasks.waker().is_set());
    tasks.start(ALICE, says("done")).expect("started");
    settle(&tasks, 1);
    // `finished` clears it, which is what the host calls.
    assert!(!tasks.waker().is_set());
}

#[test]
fn a_finished_task_makes_the_desktop_not_idle() {
    let tasks = tasks();
    assert!(!tasks.has_finished());
    let token = tasks.start(ALICE, says("hello")).expect("started");
    settle(&tasks, 1);
    // `settle` drained it; start another and look before draining.
    let again = tasks.start(ALICE, says("hello")).expect("started");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !tasks.has_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(tasks.has_finished(), "a finished task is not idle");
    let _ = (token, again);
}

#[test]
fn forgetting_a_widget_drops_its_work_rather_than_delivering_it_to_a_stranger() {
    // Widget ids are handed out in order and a suspended widget's id is not
    // reused, but the result of *its* task must not reach whatever is running
    // when it lands either way.
    let tasks = tasks();
    let token = tasks.start(ALICE, says("hers")).expect("started");
    settle(&tasks, 1);

    tasks.forget(ALICE);
    assert_eq!(tasks.take(ALICE, token), Err(TakeError::Gone));
    assert!(!tasks.has_finished(), "and nothing is left to deliver");
    assert_eq!(tasks.in_flight(ALICE), 0);
}

#[test]
fn a_task_that_finishes_after_its_widget_is_gone_is_harmless() {
    // There is no way to un-spawn a process, so this case is reachable
    // whenever a widget is suspended while a command is running.
    let tasks = tasks();
    let gate = Arc::new(std::sync::Barrier::new(2));
    let held = Arc::clone(&gate);
    tasks
        .start(
            ALICE,
            Box::new(move || {
                held.wait();
                TaskResult::Failed("late".into())
            }),
        )
        .expect("started");

    tasks.forget(ALICE);
    gate.wait();
    // Give the worker a moment to land on a token that is no longer there.
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        !tasks.has_finished(),
        "a result for a forgotten widget is dropped, not queued"
    );
}

#[test]
fn tokens_are_never_zero() {
    // Zero is what an uninitialised variable holds and what a widget sends by
    // accident. It must not name a real request.
    let tasks = tasks();
    for _ in 0..4 {
        assert_ne!(tasks.start(ALICE, says("x")).expect("started"), 0);
        settle(&tasks, 1);
    }
}

#[test]
fn a_start_error_says_what_to_do_about_it() {
    for error in [StartError::TooManyForWidget, StartError::DesktopBusy] {
        let said = error.to_string();
        assert!(said.len() > 20, "{said:?}");
    }
}

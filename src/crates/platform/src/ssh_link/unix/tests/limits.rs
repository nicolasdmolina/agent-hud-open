//! What a time limit means, and what is bounded.
use super::lab::{far, until, Lab};
use crate::ssh_link::{Cut, End, Failure, FarCommand, Link, Refused};
use std::time::{Duration, Instant};

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_command_past_its_time_limit_is_cut_off_never_ended() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let link = Link::open(&folder, &lab.target(), &lab.settings()).unwrap();
    let started = Instant::now();
    let ran = link
        .run(&FarCommand {
            limit: Duration::from_secs(1),
            ..far("echo started; sleep 30", &[])
        })
        .unwrap();
    println!("returned after {:?}", started.elapsed());
    assert_eq!(ran.end, End::CutOff(Cut::TimeLimit));
    assert_eq!(ran.stdout, b"started\n");
    assert!(started.elapsed() < Duration::from_secs(6));
    // The link is as good as before.
    assert_eq!(link.alive(), Ok(()));
    assert_eq!(link.run(&far("exit 4", &[])).unwrap().end, End::Ended(4));
    link.close();
}

/// The documented limit, not a bug to hide: a cut-off far program that
/// ignores a closed channel runs on. The wait on this side still ends on
/// time, though the far program holds the far end of its output open.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_far_command_that_ignores_hangup_survives_a_cut_off_and_nothing_waits_on_its_pipe() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let link = Link::open(&folder, &lab.target(), &lab.settings()).unwrap();
    let marker = lab.at("survived");
    let started = Instant::now();
    let ran = link
        .run(&FarCommand {
            limit: Duration::from_millis(800),
            ..far(
                "trap '' HUP PIPE TERM; echo started; sleep 4; echo late; : > \"$1\"",
                &[&marker.display().to_string()],
            )
        })
        .unwrap();
    let waited = started.elapsed();
    println!("returned after {waited:?}");
    assert_eq!(ran.end, End::CutOff(Cut::TimeLimit));
    assert_eq!(ran.stdout, b"started\n");
    assert!(waited < Duration::from_secs(3), "waited {waited:?}");
    assert!(!marker.exists(), "the far program had not finished yet");
    assert!(
        until(Duration::from_secs(20), || marker.exists()),
        "the far program was stopped by the cut-off"
    );
    println!(
        "the far program finished {:?} after it was started",
        started.elapsed()
    );
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_long_program_goes_on_standard_input_and_never_on_the_command_line() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let link = Link::open(&folder, &lab.target(), &lab.settings()).unwrap();
    let mut program = "# a line of a long program\n".repeat(2_000);
    program.push_str("printf '%s' \"got $1\"\n");
    assert!(program.len() > 50_000);
    let ran = link
        .run(&FarCommand {
            stdin: Some(program.as_bytes()),
            ..far("exec sh -s -- \"$@\"", &["it"])
        })
        .unwrap();
    assert_eq!(ran.end, End::Ended(0), "{}", ran.stderr);
    assert_eq!(ran.stdout, b"got it");
    // Standard input that is not read does not hold the wait up.
    let ran = link
        .run(&FarCommand {
            stdin: Some(program.as_bytes()),
            ..far("exit 6", &[])
        })
        .unwrap();
    assert_eq!(ran.end, End::Ended(6));

    assert_eq!(
        link.run(&far(&program, &[])).unwrap_err(),
        Failure::Refused(Refused::CommandTooLong)
    );
    assert_eq!(
        link.run(&far("cat <<END\nx\nEND", &[])).unwrap_err(),
        Failure::Refused(Refused::HereDocument)
    );
    link.close();
}

#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn what_comes_back_is_bounded() {
    let Some(lab) = Lab::start() else { return };
    let folder = lab.folder();
    let link = Link::open(&folder, &lab.target(), &lab.settings()).unwrap();
    let ran = link
        .run(&FarCommand {
            most_output: 1_000,
            ..far("head -c 300000 /dev/zero; sleep 30", &[])
        })
        .unwrap();
    assert_eq!(ran.end, End::CutOff(Cut::TooMuchOutput));
    assert_eq!(ran.stdout.len(), 1_000);
    let ran = link
        .run(&far(
            "i=0; while [ $i -lt 3000 ]; do echo 0123456789012345678901234567890123456789 >&2; i=$((i+1)); done; exit 3",
            &[],
        ))
        .unwrap();
    assert_eq!(ran.end, End::Ended(3));
    assert_eq!(ran.stderr.len(), 8 * 1024);
    link.close();
}

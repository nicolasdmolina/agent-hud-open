//! The relay's own text: what it imports, what it opens, how long it is,
//! and what it does while it runs.
use super::{
    super::RELAY,
    fixture::{chmod, private_folder},
};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    process::{Command, Stdio},
};

/// Every module the relay names, each part of Python's standard library.
const STANDARD: [&str; 13] = [
    "ctypes",
    "errno",
    "fcntl",
    "hashlib",
    "json",
    "os",
    "signal",
    "socket",
    "stat",
    "struct",
    "sys",
    "threading",
    "time",
];

/// The relay's lines of code, without what it only says in a comment or
/// its opening description.
fn code() -> Vec<&'static str> {
    let after = RELAY.splitn(3, "\"\"\"").nth(2).expect("a description");
    after
        .lines()
        .map(|line| line.split(" #").next().unwrap_or(line))
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect()
}

#[test]
fn the_relay_is_one_file_of_at_most_300_lines() {
    assert!(RELAY.lines().count() <= 300, "{}", RELAY.lines().count());
    // It starts on its last line, so a relay cut short on its way to the
    // far machine defines things and runs nothing.
    assert!(RELAY.ends_with("\nmain()\n"));
    assert_eq!(RELAY.matches("main()").count(), 2, "defined and called");
}

#[test]
fn the_relay_names_only_the_standard_library_and_opens_nothing_for_writing() {
    let code = code();
    let mut imported = Vec::new();
    for line in &code {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["import", module] => imported.push(*module),
            ["import", ..] | ["from", ..] => panic!("an import of another shape: {line}"),
            _ => assert!(!line.contains("__import__") && !line.contains("importlib")),
        }
    }
    imported.sort();
    imported.dedup();
    assert_eq!(imported, STANDARD, "what the relay imports");
    let text = code.join("\n");
    // Nothing of Hermes's is run or imported.
    for word in [
        "hermes_cli",
        "gateway.control_socket",
        "subprocess",
        "exec(",
        "eval(",
    ] {
        assert!(!text.contains(word), "{word}");
    }
    assert!(!text.contains("os.system") && !text.contains("os.popen"));
    // No file is opened but through `os.open`, and only to read.
    assert!(!text.contains(" open(") && !text.contains("=open("));
    for flag in [
        "O_WRONLY", "O_RDWR", "O_CREAT", "O_APPEND", "O_TRUNC", "O_EXCL",
    ] {
        assert!(!text.contains(flag), "{flag}");
    }
    let opens = text.matches("os.open(").count();
    let reading = text.matches("os.open(path, FOLDER)").count()
        + text
            .matches("os.open(POINTER, READ | os.O_NONBLOCK, dir_fd=home)")
            .count();
    assert_eq!((opens, reading), (2, 2), "each open is one that reads");
    assert!(text.contains("READ = os.O_RDONLY | os.O_NOFOLLOW"));
    assert!(text.contains("FOLDER = READ | getattr(os, \"O_DIRECTORY\", 0)"));
    // Nothing on the disk is made, changed or removed.
    for call in [
        "os.remove",
        "os.unlink",
        "os.rename",
        "os.replace",
        "os.mkdir",
        "os.makedirs",
        "os.rmdir",
        "os.chmod",
        "os.chown",
        "os.fchmod",
        "os.fchown",
        "os.truncate",
        "os.symlink",
        "os.link",
        "os.utime",
        "os.mkfifo",
        "shutil",
        "tempfile",
        ".bind(",
    ] {
        assert!(!text.contains(call), "{call}");
    }
    // The only write is to standard output.
    assert_eq!(text.matches("os.write(").count(), 1);
    assert!(text.contains("os.write(1, data)"));
}

/// The same, watched while it runs: Python tells an audit hook of every
/// file it opens and every module it loads.
#[test]
#[ignore = "in the lab: tests::the_lab_tests_pass runs it"]
fn a_running_relay_loads_only_the_standard_library_and_opens_nothing_for_writing() {
    let dir = tempfile::Builder::new()
        .prefix("ahr")
        .tempdir_in("/tmp")
        .unwrap();
    let home = fs::canonicalize(dir.path()).unwrap().join("h");
    private_folder(&home);
    let listener = UnixListener::bind(home.join("gateway.sock")).unwrap();
    chmod(&home.join("gateway.sock"), 0o600);
    let serving = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        (&stream).write_all(b"{\"ok\": true}\n").unwrap();
        line
    });
    // In front of the relay: a hook that keeps what it is told, and prints
    // it to standard error when the relay ends itself.
    let watch = "import os as _os, sys as _sys\n\
        _seen = []\n\
        def _hook(event, args):\n\
        \x20   if event == 'open':\n\
        \x20       _seen.append('open\\t%r\\t%r\\t%r' % (args[0], args[1], args[2]))\n\
        \x20   elif event == 'import':\n\
        \x20       _seen.append('import\\t%s\\t%s' % (args[0], args[1]))\n\
        \x20   elif event == 'os.exit' or event.startswith(('os.remove', 'os.rename', 'os.mkdir', \
        'os.rmdir', 'os.chmod', 'os.chown', 'os.truncate', 'os.symlink', 'os.link', 'subprocess', \
        'os.exec', 'os.fork', 'os.posix_spawn', 'os.system', 'shutil')):\n\
        \x20       _seen.append('did\\t' + event)\n\
        _sys.addaudithook(_hook)\n\
        _real_exit = _os._exit\n\
        def _exit(code):\n\
        \x20   _os.write(2, ('\\n'.join(_seen) + '\\n').encode())\n\
        \x20   _real_exit(code)\n\
        _os._exit = _exit\n";
    let mut relay = Command::new("/usr/bin/python3")
        .args(["-I", "-S", "-B", "-", "5", home.to_str().unwrap()])
        .arg("{\"protocol\": 1, \"id\": 1, \"verb\": \"identify\"}")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python3");
    let mut program = relay.stdin.take().unwrap();
    program.write_all(watch.as_bytes()).unwrap();
    program.write_all(RELAY.as_bytes()).unwrap();
    drop(program);
    let ran = relay.wait_with_output().unwrap();
    assert_eq!(
        serving.join().unwrap().trim(),
        "{\"protocol\": 1, \"id\": 1, \"verb\": \"identify\"}"
    );
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "ok\n{\"ok\": true}\n");
    let seen = String::from_utf8_lossy(&ran.stderr).into_owned();
    println!("{seen}");
    // A file of Python's own library, and not a package added to it.
    let library = |path: &str| path.contains("/lib/python3") && !path.contains("-packages");
    let (mut opened, mut loaded, mut own) = (0, 0, 0);
    for line in seen.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        match parts.as_slice() {
            ["open", path, mode, flags] => {
                opened += 1;
                let flags: i32 = flags.parse().expect("the flags");
                let writing = libc::O_WRONLY | libc::O_RDWR | libc::O_CREAT | libc::O_TRUNC;
                assert_eq!(flags & (writing | libc::O_APPEND), 0, "{line}");
                // A mode is given by Python's own `open`, which reads its
                // library; the relay's opens carry flags alone, and are of
                // the home and what is in it.
                match *mode {
                    "'r'" | "'rb'" => assert!(library(path), "{line}"),
                    "None" => {
                        own += 1;
                        assert!(path.contains(home.to_str().unwrap()), "{line}");
                    }
                    _ => panic!("opened another way: {line}"),
                }
            }
            ["import", module, from] => {
                loaded += 1;
                assert!(
                    *from == "None" || library(from),
                    "{module} was loaded from {from}"
                );
            }
            ["did", "os.exit"] => {}
            _ => panic!("the relay did something else: {line}"),
        }
    }
    // The relay itself opened the home and nothing else: the socket in it
    // was found from that descriptor. The rest is Python reading its library.
    assert!(
        opened > own && loaded >= 5,
        "{opened} opened, {loaded} loaded"
    );
    assert_eq!(own, 1, "the relay's own opens");
    assert!(
        seen.contains(&format!("open\t'{}'\tNone", home.display())),
        "the home"
    );
}

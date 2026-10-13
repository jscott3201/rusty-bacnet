# Contributing to Rusty BACnet

Bug reports, test cases, documentation fixes and focused patches are welcome.
Issues, pull requests and CI all live on
[GitHub](https://github.com/jscott3201/rusty-bacnet).

- **Problems and ideas:** open an
  [issue](https://github.com/jscott3201/rusty-bacnet/issues/new/choose) with
  the bug report or feature request form. A packet capture (PCAP) of the
  exchange is welcome; leave out credentials and anything from a real network
  you don't want public.
- **Where the project is going:** the public
  [Rusty BACnet Roadmap](https://github.com/users/jscott3201/projects/4)
  board shows every open issue by track, priority and target release. Its
  **Good first issues** and **Needs decision** views are good places to start.
- **Security vulnerabilities:** report them privately, as the
  [security policy](.github/SECURITY.md) describes, not in a public issue.

## Build and test

`rust-toolchain.toml` pins the Rust release, which rustup selects in the
checkout; published crates keep an MSRV of 1.93. Tests run with
[cargo-nextest](https://nexte.st) 0.9.145 or later, which skips doctests, so
run `cargo test --doc` as well. The block below picks the features CI uses on
your OS. On Linux, install a C toolchain, `pkg-config` and `libpcap-dev` first
(Debian and Ubuntu names), because the list includes the CLI's packet capture;
the Python bindings also need `python3-venv` for maturin's venv and
`libpython3-dev` for the PyO3 crate's tests. Windows leaves out
`bacnet-cli/pcap`, which needs the Npcap SDK.

```bash
case "$(uname -s)" in
  Linux)  FEATURES=$(sed -n 's/^  LINUX_FEATURES: //p' .github/workflows/ci.yml) ;;  # every optional feature
  Darwin) FEATURES=$(sed -n 's/^features=//p' scripts/ci/local-macos.sh) ;;          # all but serial and ethernet
  *)      FEATURES=$(sed -n 's/^features=//p' scripts/ci/local-macos.sh | sed 's#,bacnet-cli/pcap##') ;;  # Windows (Git Bash)
esac

cargo fmt --all --check
cargo clippy --workspace --exclude rusty-bacnet --all-targets --locked --features "$FEATURES" -- -D warnings
cargo nextest run --workspace --exclude rusty-bacnet --locked --features "$FEATURES"
cargo test --doc --workspace --exclude rusty-bacnet --locked --features "$FEATURES"
```

For a quicker loop, test only the crate you changed and the crates that
depend on it (`cargo nextest run -p bacnet-server`). On a Mac,
`bash scripts/ci/local-macos.sh` runs the whole gate. The Python bindings
(`crates/rusty-bacnet`) build with maturin; the [README](README.md#contributing)
has those steps. [docs/ci.md](docs/ci.md#local-checks) lists every check CI
runs, with the commands.

Clippy and rustdoc deny warnings, every public item needs a doc comment, and a
Rust source file may hold at most 700 lines of code
(`bash scripts/ci/check-file-size.sh`).

## Pull requests

- Branch from `dev` and open the pull request against `dev`; `main` only takes
  releases.
- Keep each pull request to one outcome, and link its issue (`Closes #123`).
- Add or update tests for any behaviour you change.
- Before 1.0.0 the APIs aren't frozen. A breaking change updates every caller,
  test, example and doc in the repository and says what breaks.
- A change users can see adds one fragment to `changelog.d/` instead of
  editing `CHANGELOG.md`. [changelog.d/README.md](changelog.d/README.md) gives
  the format; check it with `python3 scripts/changelog.py check`.
- A pull request merges when the required checks `CI OK` (Linux) and
  `Native OK` (macOS and Windows) pass on its latest commit and a maintainer
  has reviewed it. A first-time contributor's workflow runs may wait for a
  maintainer's approval to start.

## ASHRAE Standard 135

The project implements ASHRAE Standard 135-2020, which is copyrighted and
licensed by ASHRAE, and the repository doesn't include it. When code comments,
docs, issues or pull requests depend on what the standard says, describe the
behaviour in your own words and cite the clause number, such as
"(Clause 13.1)". Never paste text, tables or figures from the standard.
Property, object and service names such as `Present_Value` are fine.

## More

- [docs/ci.md](docs/ci.md): the CI pipeline, the merge checks and the release
  process.
- [AGENTS.md](AGENTS.md): the repository's working rules, written for coding
  agents.
- The [documentation guide](https://jscott3201.github.io/rusty-bacnet/project/contributing/)
  for website and docs changes.

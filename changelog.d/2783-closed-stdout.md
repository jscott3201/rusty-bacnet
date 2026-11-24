---
section: Fixed
---
- **CLI:** a closed stdout (`| head -1`) stops `subscribe` and `capture`
  quietly with status 0, a finite subscription cancelled, where they panicked
  with status 101; `capture` stops on SIGTERM and SIGHUP as on Ctrl+C, with
  its summary, instead of being killed (#2783).

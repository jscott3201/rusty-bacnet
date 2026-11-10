---
section: Fixed
---
- A write to a Timer's State_Change_Values holding a REAL or Double that
  is not a number is saved once, off the database lock, like any other
  kept write. It was saved three times, twice with the database held
  (#2331).

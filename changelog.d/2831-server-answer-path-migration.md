---
section: Migration notes
---
- **Server answer path (wire, #2831):** where a remote network's answers come
  back through another router than the server's requests go through, those
  requests now time out; watch `off_path_answer_drops`. A learned router is
  kept while it answers and forgotten once it stays silent, then learned again
  from the next broadcast attempt's answer.

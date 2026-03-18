window.BENCHMARK_DATA = {
  "lastUpdate": 1773802467010,
  "repoUrl": "https://github.com/halcyonnouveau/soppo",
  "entries": {
    "Benchmark": [
      {
        "commit": {
          "author": {
            "email": "justin@duch.me",
            "name": "Justin Duch",
            "username": "beanpuppy"
          },
          "committer": {
            "email": "justin@duch.me",
            "name": "Justin Duch",
            "username": "beanpuppy"
          },
          "distinct": true,
          "id": "e85b4909b2ceb12e703e6fe10eb5e6d0b2f1139e",
          "message": "feat; add runtime benchmarks",
          "timestamp": "2026-03-18T13:49:45+11:00",
          "tree_id": "44f1c81d96f93993b22701ebc00dac3bd5661ff0",
          "url": "https://github.com/halcyonnouveau/soppo/commit/e85b4909b2ceb12e703e6fe10eb5e6d0b2f1139e"
        },
        "date": 1773802466006,
        "tool": "cargo",
        "benches": [
          {
            "name": "parse/file/guessing_game",
            "value": 119827,
            "range": "± 651",
            "unit": "ns/iter"
          },
          {
            "name": "parse/file/file_processor",
            "value": 263149,
            "range": "± 3679",
            "unit": "ns/iter"
          },
          {
            "name": "parse/file/http_server",
            "value": 225596,
            "range": "± 9247",
            "unit": "ns/iter"
          },
          {
            "name": "parse/file/todo_cli",
            "value": 1647920,
            "range": "± 26560",
            "unit": "ns/iter"
          },
          {
            "name": "parse/file/basic_go",
            "value": 18356008,
            "range": "± 108343",
            "unit": "ns/iter"
          },
          {
            "name": "typecheck/file/guessing_game",
            "value": 2236821,
            "range": "± 13685",
            "unit": "ns/iter"
          },
          {
            "name": "typecheck/file/file_processor",
            "value": 4059599,
            "range": "± 160786",
            "unit": "ns/iter"
          },
          {
            "name": "typecheck/file/http_server",
            "value": 8986715,
            "range": "± 43044",
            "unit": "ns/iter"
          },
          {
            "name": "typecheck/file/todo_cli",
            "value": 4968974,
            "range": "± 64500",
            "unit": "ns/iter"
          },
          {
            "name": "typecheck/file/basic_go",
            "value": 25175660,
            "range": "± 61044",
            "unit": "ns/iter"
          },
          {
            "name": "compile/file/guessing_game",
            "value": 2280340,
            "range": "± 13159",
            "unit": "ns/iter"
          },
          {
            "name": "compile/file/file_processor",
            "value": 4142723,
            "range": "± 125059",
            "unit": "ns/iter"
          },
          {
            "name": "compile/file/http_server",
            "value": 9086586,
            "range": "± 46603",
            "unit": "ns/iter"
          },
          {
            "name": "compile/file/todo_cli",
            "value": 4999305,
            "range": "± 153085",
            "unit": "ns/iter"
          },
          {
            "name": "compile/file/basic_go",
            "value": 25219149,
            "range": "± 103156",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}
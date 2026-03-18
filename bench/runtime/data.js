window.BENCHMARK_DATA = {
  "lastUpdate": 1773802782498,
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
        "date": 1773802256275,
        "tool": "go",
        "benches": [
          {
            "name": "BenchmarkEnumMatchBaseline",
            "value": 8.122,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "147423049 times\n4 procs"
          },
          {
            "name": "BenchmarkEnumMatchBaseline - ns/op",
            "value": 8.122,
            "unit": "ns/op",
            "extra": "147423049 times\n4 procs"
          },
          {
            "name": "BenchmarkEnumMatchBaseline - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "147423049 times\n4 procs"
          },
          {
            "name": "BenchmarkEnumMatchBaseline - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "147423049 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapBaseline",
            "value": 0.7808,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "1000000000 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapBaseline - ns/op",
            "value": 0.7808,
            "unit": "ns/op",
            "extra": "1000000000 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapBaseline - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "1000000000 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapBaseline - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "1000000000 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideBaseline",
            "value": 1.556,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "771097258 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideBaseline - ns/op",
            "value": 1.556,
            "unit": "ns/op",
            "extra": "771097258 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideBaseline - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "771097258 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideBaseline - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "771097258 times\n4 procs"
          },
          {
            "name": "BenchmarkEnumMatchGenerated",
            "value": 4.76,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "251929898 times\n4 procs"
          },
          {
            "name": "BenchmarkEnumMatchGenerated - ns/op",
            "value": 4.76,
            "unit": "ns/op",
            "extra": "251929898 times\n4 procs"
          },
          {
            "name": "BenchmarkEnumMatchGenerated - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "251929898 times\n4 procs"
          },
          {
            "name": "BenchmarkEnumMatchGenerated - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "251929898 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapGenerated",
            "value": 1.559,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "770127735 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapGenerated - ns/op",
            "value": 1.559,
            "unit": "ns/op",
            "extra": "770127735 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapGenerated - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "770127735 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapGenerated - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "770127735 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideGenerated",
            "value": 2.496,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "480424342 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideGenerated - ns/op",
            "value": 2.496,
            "unit": "ns/op",
            "extra": "480424342 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideGenerated - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "480424342 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideGenerated - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "480424342 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateBaseline",
            "value": 1.869,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "641727663 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateBaseline - ns/op",
            "value": 1.869,
            "unit": "ns/op",
            "extra": "641727663 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateBaseline - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "641727663 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateBaseline - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "641727663 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorBaseline",
            "value": 1.256,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "899614836 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorBaseline - ns/op",
            "value": 1.256,
            "unit": "ns/op",
            "extra": "899614836 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorBaseline - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "899614836 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorBaseline - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "899614836 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedBaseline",
            "value": 3.426,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "350268014 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedBaseline - ns/op",
            "value": 3.426,
            "unit": "ns/op",
            "extra": "350268014 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedBaseline - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "350268014 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedBaseline - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "350268014 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateGenerated",
            "value": 1.872,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "640317596 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateGenerated - ns/op",
            "value": 1.872,
            "unit": "ns/op",
            "extra": "640317596 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateGenerated - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "640317596 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateGenerated - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "640317596 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorGenerated",
            "value": 1.248,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "960822848 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorGenerated - ns/op",
            "value": 1.248,
            "unit": "ns/op",
            "extra": "960822848 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorGenerated - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "960822848 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorGenerated - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "960822848 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedGenerated",
            "value": 3.455,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "348724578 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedGenerated - ns/op",
            "value": 3.455,
            "unit": "ns/op",
            "extra": "348724578 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedGenerated - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "348724578 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedGenerated - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "348724578 times\n4 procs"
          }
        ]
      },
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
          "id": "ab96432c735ab109988cf54b3fae57da27eff15b",
          "message": "feat; add var block",
          "timestamp": "2026-03-18T13:58:48+11:00",
          "tree_id": "b4fbe9d3b09738cc2af8cecf45f9b85671c6dc09",
          "url": "https://github.com/halcyonnouveau/soppo/commit/ab96432c735ab109988cf54b3fae57da27eff15b"
        },
        "date": 1773802781974,
        "tool": "go",
        "benches": [
          {
            "name": "BenchmarkEnumMatchBaseline",
            "value": 7.912,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "151674488 times\n4 procs"
          },
          {
            "name": "BenchmarkEnumMatchBaseline - ns/op",
            "value": 7.912,
            "unit": "ns/op",
            "extra": "151674488 times\n4 procs"
          },
          {
            "name": "BenchmarkEnumMatchBaseline - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "151674488 times\n4 procs"
          },
          {
            "name": "BenchmarkEnumMatchBaseline - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "151674488 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapBaseline",
            "value": 0.828,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "1000000000 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapBaseline - ns/op",
            "value": 0.828,
            "unit": "ns/op",
            "extra": "1000000000 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapBaseline - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "1000000000 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapBaseline - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "1000000000 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideBaseline",
            "value": 1.573,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "770566312 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideBaseline - ns/op",
            "value": 1.573,
            "unit": "ns/op",
            "extra": "770566312 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideBaseline - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "770566312 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideBaseline - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "770566312 times\n4 procs"
          },
          {
            "name": "BenchmarkEnumMatchGenerated",
            "value": 4.95,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "235880018 times\n4 procs"
          },
          {
            "name": "BenchmarkEnumMatchGenerated - ns/op",
            "value": 4.95,
            "unit": "ns/op",
            "extra": "235880018 times\n4 procs"
          },
          {
            "name": "BenchmarkEnumMatchGenerated - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "235880018 times\n4 procs"
          },
          {
            "name": "BenchmarkEnumMatchGenerated - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "235880018 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapGenerated",
            "value": 1.571,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "764862681 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapGenerated - ns/op",
            "value": 1.571,
            "unit": "ns/op",
            "extra": "764862681 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapGenerated - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "764862681 times\n4 procs"
          },
          {
            "name": "BenchmarkResultUnwrapGenerated - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "764862681 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideGenerated",
            "value": 2.501,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "479099072 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideGenerated - ns/op",
            "value": 2.501,
            "unit": "ns/op",
            "extra": "479099072 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideGenerated - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "479099072 times\n4 procs"
          },
          {
            "name": "BenchmarkOptionDivideGenerated - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "479099072 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateBaseline",
            "value": 1.871,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "641434828 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateBaseline - ns/op",
            "value": 1.871,
            "unit": "ns/op",
            "extra": "641434828 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateBaseline - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "641434828 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateBaseline - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "641434828 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorBaseline",
            "value": 1.251,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "931359835 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorBaseline - ns/op",
            "value": 1.251,
            "unit": "ns/op",
            "extra": "931359835 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorBaseline - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "931359835 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorBaseline - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "931359835 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedBaseline",
            "value": 3.429,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "349619137 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedBaseline - ns/op",
            "value": 3.429,
            "unit": "ns/op",
            "extra": "349619137 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedBaseline - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "349619137 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedBaseline - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "349619137 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateGenerated",
            "value": 1.87,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "641913088 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateGenerated - ns/op",
            "value": 1.87,
            "unit": "ns/op",
            "extra": "641913088 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateGenerated - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "641913088 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateGenerated - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "641913088 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorGenerated",
            "value": 1.25,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "960997606 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorGenerated - ns/op",
            "value": 1.25,
            "unit": "ns/op",
            "extra": "960997606 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorGenerated - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "960997606 times\n4 procs"
          },
          {
            "name": "BenchmarkTryPropagateErrorGenerated - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "960997606 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedGenerated",
            "value": 3.429,
            "unit": "ns/op\t       0 B/op\t       0 allocs/op",
            "extra": "349873710 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedGenerated - ns/op",
            "value": 3.429,
            "unit": "ns/op",
            "extra": "349873710 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedGenerated - B/op",
            "value": 0,
            "unit": "B/op",
            "extra": "349873710 times\n4 procs"
          },
          {
            "name": "BenchmarkTryWrappedGenerated - allocs/op",
            "value": 0,
            "unit": "allocs/op",
            "extra": "349873710 times\n4 procs"
          }
        ]
      }
    ]
  }
}
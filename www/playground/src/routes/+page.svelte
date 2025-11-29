<script lang="ts">
  import Editor from "$lib/components/Editor.svelte";

  const EXAMPLES: Record<string, string> = {
    "Enums & Pattern Matching": `package main

import "fmt"

type Colour enum {
    Red
    Yellow
    Green
}

type Shape enum {
    Circle struct {
        radius float64
    }
    Rectangle struct {
        width  float64
        height float64
    }
}

func area(s Shape) float64 {
    var result float64
    match s {
    case Shape.Circle{radius: r, ...}:
        result = 3.14 * r * r
    case Shape.Rectangle{width: w, height: h}:
        result = w * h
    }
    return result
}

func main() {
    light := Colour.Red

    match light {
    case Colour.Red:
        fmt.Println("Stop")
    case Colour.Yellow:
        fmt.Println("Caution")
    case Colour.Green:
        fmt.Println("Go")
    }

    circle := Shape.Circle{radius: 5.0}
    rect := Shape.Rectangle{width: 3.0, height: 4.0}

    fmt.Println("Circle area:", area(circle))
    fmt.Println("Rectangle area:", area(rect))
}
`,
    Generics: `package main

import "fmt"

type Option[T any] enum {
    Some T
    None
}

type Result[T any, E any] enum {
    Ok T
    Err E
}

func main() {
    x := Option.Some(42)
    y := Option.Some("hello")
    z := Option.Some[int](100)

    r1 := Result.Ok[int, string](1)
    r2 := Result.Err[int, string]("failed")

    fmt.Printf("%v %v %v %v %v\\n", x, y, z, r1, r2)
}
`,
    "String Interpolation": `package main

import "fmt"

func main() {
    name := "World"
    fmt.Println("Hello, {name}!")

    x := 10
    y := 20
    fmt.Println("x={x}, y={y}, sum={x + y}")

    nums := []int{1, 2, 3}
    fmt.Println("Length: {len(nums)}")

    a := 5
    b := 3
    fmt.Println("Result: {a * b + 2}")

    // Escaped braces
    fmt.Println("Use {{name}} for interpolation")

    pct := 50
    fmt.Println("{pct}% complete")

    pi := 3.14159
    fmt.Println("Pi is approximately {pi}")
}
`,
    "Error Handling (? operator)": `package main

import "fmt"
import "errors"

func parsePort(s string) (int, error) {
    if s == "" {
        return 0, errors.New("empty string")
    }
    return 8080, nil
}

func getUser(id int) (*struct{ name string }, error) {
    if id <= 0 {
        return nil, errors.New("invalid id")
    }
    return &struct{ name string }{name: "Alice"}, nil
}

func process() error {
    // Simple propagation
    port := parsePort("8080") ?
    fmt.Println("Port:", port)

    // With custom error handler
    user := getUser(1) ? err {
        return fmt.Errorf("failed to get user: %v", err)
    }
    fmt.Println("User:", user.name)

    return nil
}

func main() {
    if err := process(); err != nil {
        fmt.Println("Error:", err)
    }
}
`,
    "Nil Safety": `package main

import "fmt"

type User struct {
    name    string
    profile *struct {
        bio string
    }
}

func getUser(id int) *User {
    if id == 1 {
        return &User{name: "Alice", profile: &struct{ bio string }{bio: "Hello!"}}
    }
    return nil
}

func main() {
    user := getUser(1)

    // After nil check, user is known to be non-nil
    if user != nil {
        fmt.Println("Name:", user.name)

        if user.profile != nil {
            fmt.Println("Bio:", user.profile.bio)
        }
    }

    // Early return pattern
    user2 := getUser(0)
    if user2 == nil {
        fmt.Println("User not found")
        return
    }
    fmt.Println(user2.name)
}
`,
    "Goroutines & Channels": `package main

import "fmt"
import "time"

type Result enum {
    Ok int
    Err string
}

func worker(id int, jobs chan int, results chan Result) {
    for j := range jobs {
        fmt.Println("worker {id} processing job {j}")
        time.Sleep(time.Millisecond * 100)

        match {
        case j % 3 == 0:
            results <- Result.Err("job {j} failed")
        default:
            results <- Result.Ok(j * 2)
        }
    }
}

func main() {
    const numJobs = 5
    const numWorkers = 2

    jobs := make(chan int, numJobs)
    results := make(chan Result, numJobs)

    for w := 1; w <= numWorkers; w++ {
        go worker(id: w, jobs: jobs, results: results)
    }

    for j := 1; j <= numJobs; j++ {
        jobs <- j
    }
    close(jobs)

    for i := 1; i <= numJobs; i++ {
        result := <-results
        match result {
        case Result.Ok(value):
            fmt.Println("success: {value}")
        case Result.Err(msg):
            fmt.Println("error: {msg}")
        }
    }
}
`,
  };

  const exampleNames = Object.keys(EXAMPLES);
  const firstExample = exampleNames[0];
  let selectedExample = $state(firstExample);
  let source = $state(EXAMPLES[firstExample]);
  let goCode = $state("");
  let output = $state("");
  let error = $state("");
  let isLoading = $state(false);
  let vimMode = $state(false);

  function selectExample(name: string) {
    selectedExample = name;
    source = EXAMPLES[name];
    goCode = "";
    output = "";
    error = "";
  }

  async function compile() {
    isLoading = true;
    error = "";
    output = "";
    goCode = "";

    try {
      const response = await fetch("/api/compile", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ source }),
      });

      const result = await response.json();

      if (result.compileError) {
        error = result.compileError;
      } else {
        goCode = result.goCode || "";
        if (result.runError) {
          error = result.runError;
        } else {
          output = result.output || "";
        }
      }
    } catch (err) {
      error = `Request failed: ${err}`;
    } finally {
      isLoading = false;
    }
  }
</script>

<svelte:head>
  <title>Soppo Playground</title>
</svelte:head>

<div class="container">
  <header>
    <div class="header-left">
      <h1>Soppo Playground</h1>
      <select
        value={selectedExample}
        onchange={(e) => selectExample(e.currentTarget.value)}
      >
        {#each exampleNames as name}
          <option value={name}>{name}</option>
        {/each}
      </select>
    </div>
    <div class="header-right">
      <a
        href="https://github.com/halcyonnouveau/soppo"
        target="_blank"
        rel="noopener"
        class="github-link"
        title="View on GitHub"
      >
        <svg viewBox="0 0 16 16" width="16" height="16" fill="currentColor">
          <path
            d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"
          />
        </svg>
        Soppo on GitHub
      </a>
      <button onclick={compile} disabled={isLoading}>
        {isLoading ? "Running..." : "Run"}
      </button>
    </div>
  </header>

  <main>
    <div class="editors">
      <div class="panel">
        <div class="panel-header">
          <span>Soppo</span>
          <label class="vim-toggle">
            <input type="checkbox" bind:checked={vimMode} />
            Vim mode
          </label>
        </div>
        <div class="panel-content">
          <Editor bind:value={source} {vimMode} onrun={compile} />
        </div>
      </div>

      <div class="panel">
        <div class="panel-header">Generated Go</div>
        <div class="panel-content">
          <Editor value={goCode} readonly {vimMode} />
        </div>
      </div>
    </div>

    <div class="panel output-panel">
      <div class="panel-header">Output</div>
      <div class="panel-content">
        {#if error}
          <pre class="error">{error}</pre>
        {:else}
          <pre>{output}</pre>
        {/if}
      </div>
    </div>
  </main>
</div>

<style>
  :global(body) {
    margin: 0;
    font-family:
      system-ui,
      -apple-system,
      sans-serif;
    background: #1e1e1e;
    color: #d4d4d4;
  }

  .container {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.5rem 1rem;
    background: #252526;
    border-bottom: 1px solid #3c3c3c;
    gap: 1rem;
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 1rem;
  }

  .header-right {
    display: flex;
    align-items: center;
    gap: 1rem;
  }

  .github-link {
    color: #aaa;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    text-decoration: none;
    font-size: 1rem;
  }

  .github-link:hover {
    color: #fff;
  }

  h1 {
    margin: 0;
    font-size: 1.25rem;
    font-weight: 500;
  }

  select {
    background: #3c3c3c;
    color: #d4d4d4;
    border: 1px solid #555;
    padding: 0.4rem 0.75rem;
    border-radius: 4px;
    font-size: 0.875rem;
    cursor: pointer;
  }

  select:hover {
    background: #4a4a4a;
  }

  .vim-toggle {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.75rem;
    text-transform: none;
    cursor: pointer;
    user-select: none;
  }

  .vim-toggle input {
    cursor: pointer;
  }

  button {
    background: #0e639c;
    color: white;
    border: none;
    padding: 0.5rem 1.5rem;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.875rem;
  }

  button:hover:not(:disabled) {
    background: #1177bb;
  }

  button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  main {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
  }

  .editors {
    display: grid;
    grid-template-columns: 1fr 1fr;
    flex: 1;
    overflow: hidden;
  }

  .panel {
    display: flex;
    flex-direction: column;
    border-right: 1px solid #3c3c3c;
    overflow: hidden;
  }

  .panel:last-child {
    border-right: none;
  }

  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 1rem;
    height: 2rem;
    background: #2d2d2d;
    border-bottom: 1px solid #3c3c3c;
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #888;
  }

  .panel-content {
    flex: 1;
    overflow: auto;
  }

  .output-panel {
    height: 300px;
    min-height: 200px;
    border-top: 1px solid #3c3c3c;
  }

  pre {
    margin: 0;
    padding: 1rem;
    font-family: "SF Mono", Monaco, "Cascadia Code", monospace;
    font-size: 0.875rem;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .error {
    color: #f48771;
  }
</style>

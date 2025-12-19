<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { getLesson, lessons } from "$lib/lessons";
  import Editor from "$lib/components/Editor.svelte";

  let lesson = $derived(getLesson(page.params.lesson || "/"));
  let code = $state("");
  let output = $state("");
  let goCode = $state("");
  let error = $state("");
  let isLoading = $state(false);
  let activeTab: "output" | "go" = $state("output");

  let debounceTimer: ReturnType<typeof setTimeout>;

  $effect(() => {
    if (lesson) {
      code = lesson.code;
      output = "";
      goCode = "";
      error = "";
    }
  });

  $effect(() => {
    if (!lesson && lessons.length > 0) {
      goto(`/${lessons[0].slug}`, { replaceState: true });
    }
  });

  function handleCodeChange(newCode: string) {
    code = newCode;
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      compile();
    }, 500);
  }

  async function compile() {
    if (!code.trim()) return;

    isLoading = true;
    error = "";

    try {
      const response = await fetch("/api/compile", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ source: code }),
      });

      const result = await response.json();

      if (result.compileError) {
        error = result.compileError;
        goCode = "";
      } else {
        goCode = result.goCode || "";
        if (result.runError) {
          error = result.runError;
        } else {
          output = result.output || "";
          error = "";
        }
      }
    } catch (err) {
      error = `Request failed: ${err}`;
    } finally {
      isLoading = false;
    }
  }
</script>

{#if lesson}
  <div class="lesson">
    <div class="left-panel">
      <div class="lesson-content">
        <h1>{lesson.title}</h1>
        {#each lesson.content.split("\n\n") as paragraph}
          {#if paragraph.startsWith("**")}
            <p class="task">
              {@html paragraph
                .replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>")
                .replace(/`([^`]+)`/g, "<code>$1</code>")}
            </p>
          {:else}
            <p>{@html paragraph.replace(/`([^`]+)`/g, "<code>$1</code>")}</p>
          {/if}
        {/each}
      </div>
    </div>

    <div class="right-panel">
      <div class="editor-wrapper">
        <Editor value={code} onchange={handleCodeChange} />
      </div>

      <div class="output-panel">
        <div class="tabs">
          <button
            class="tab"
            class:active={activeTab === "output"}
            onclick={() => (activeTab = "output")}
          >
            Output
          </button>
          <button
            class="tab"
            class:active={activeTab === "go"}
            disabled={!goCode}
            onclick={() => (activeTab = "go")}
          >
            Generated Go
          </button>
          {#if isLoading}
            <span class="loading">Running...</span>
          {/if}
        </div>
        <div class="output-content" class:go-view={activeTab === "go"}>
          {#if activeTab === "output"}
            {#if error}
              <pre class="error">{error}</pre>
            {:else}
              <pre>{output}</pre>
            {/if}
          {:else}
            <Editor value={goCode} readonly />
          {/if}
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .lesson {
    display: flex;
    height: 100%;
    gap: 2rem;
    padding: 0 2rem;
  }

  .left-panel {
    width: 38%;
    padding: 1rem 0;
    overflow-y: auto;
  }

  .lesson-content h1 {
    font-size: 1.5rem;
    font-weight: normal;
    margin-bottom: 1.25rem;
    color: #e8e4df;
  }

  .lesson-content p {
    color: #a8a4a0;
    margin-bottom: 1rem;
    font-size: 0.95rem;
  }

  .lesson-content .task {
    color: #c8c4bf;
    background: rgba(78, 201, 176, 0.08);
    padding: 0.875rem 1rem;
    border-left: 2px solid #4ec9b0;
    margin-top: 1.25rem;
  }

  .lesson-content :global(strong) {
    color: #e8e4df;
    font-weight: 600;
  }

  .lesson-content :global(code) {
    background: rgba(255, 255, 255, 0.06);
    padding: 0.1rem 0.35rem;
    font-size: 0.85em;
  }

  .right-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-width: 0;
  }

  .editor-wrapper {
    flex: 1;
    overflow: hidden;
    min-height: 0;
  }

  .output-panel {
    height: 280px;
    display: flex;
    flex-direction: column;
    margin-top: 0.75rem;
  }

  .tabs {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding-bottom: 0.5rem;
  }

  .loading {
    margin-left: auto;
    color: #4ec9b0;
    font-size: 0.8rem;
  }

  .tab {
    background: none;
    border: none;
    color: #5a5550;
    padding: 0.35rem 0.75rem;
    font-family: Georgia, "Times New Roman", serif;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .tab:hover:not(:disabled) {
    color: #8a8480;
  }

  .tab.active {
    color: #e8e4df;
    background: rgba(255, 255, 255, 0.06);
  }

  .tab:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }

  .output-content {
    flex: 1;
    overflow: auto;
    padding: 1rem;
    background: #0d0d0d;
  }

  .output-content.go-view {
    padding: 0;
  }

  .output-content pre {
    font-size: 13px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
    color: #b8b4af;
  }

  .error {
    color: #f48771;
  }

  @media (max-width: 900px) {
    .lesson {
      flex-direction: column;
      height: auto;
      gap: 1rem;
      padding: 0 1rem 1rem;
    }

    .left-panel {
      width: 100%;
      padding: 0.5rem 0;
      overflow: visible;
    }

    .lesson-content h1 {
      font-size: 1.35rem;
    }

    .right-panel {
      height: auto;
    }

    .editor-wrapper {
      height: 300px;
    }

    .output-panel {
      height: 200px;
    }
  }
</style>

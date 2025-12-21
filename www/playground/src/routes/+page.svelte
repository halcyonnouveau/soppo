<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import LZString from "lz-string";
  import Editor from "$lib/components/Editor.svelte";

  const defaultCode = `package main

import "fmt"

func main() {
	fmt.Println("Hello, Soppo!")
}
`;

  let source = $state(defaultCode);
  let goCode = $state("");
  let output = $state("");
  let error = $state("");
  let isLoading = $state(false);
  let vimMode = $state(false);
  let activeTab: "output" | "go" = $state("output");
  let shareText = $state("Share");

  // Load code from URL on mount
  onMount(() => {
    const codeParam = page.url.searchParams.get("code");
    if (codeParam) {
      let decoded = LZString.decompressFromEncodedURIComponent(codeParam);
      if (!decoded) {
        // Fallback for old base64 URLs
        try {
          decoded = decodeURIComponent(atob(codeParam));
        } catch {}
      }
      if (decoded) {
        source = decoded;
      }
    }
  });

  function share() {
    const encoded = LZString.compressToEncodedURIComponent(source);
    const url = `${window.location.origin}${window.location.pathname}?code=${encoded}`;
    navigator.clipboard.writeText(url);
    shareText = "Copied!";
    setTimeout(() => {
      shareText = "Share";
    }, 2000);
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

<div class="app">
  <header>
    <a href="https://soppolang.dev" class="logo">Soppo Playground</a>
    <div class="header-right">
      <label class="vim-toggle">
        <input type="checkbox" bind:checked={vimMode} />
        Vim
      </label>
      <button class="header-btn" onclick={compile} disabled={isLoading}>
        {isLoading ? "Running..." : "Run"}
      </button>
      <button class="header-btn" onclick={share}>
        {shareText}
      </button>
      <a href="https://github.com/halcyonnouveau/soppo" aria-label="GitHub">
        <svg viewBox="0 0 16 16" width="18" height="18" fill="currentColor">
          <path
            d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"
          />
        </svg>
      </a>
    </div>
  </header>

  <main>
    <div class="left-panel">
      <Editor bind:value={source} {vimMode} onrun={compile} />
    </div>

    <div class="right-panel">
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
          <Editor value={goCode} readonly {vimMode} />
        {/if}
      </div>
    </div>
  </main>
</div>

<style>
  :global(*) {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
  }

  :global(html),
  :global(body) {
    height: 100%;
    overflow: hidden;
  }

  :global(body) {
    background: #1a1a1a;
    color: #e8e4df;
    font-family: Georgia, "Times New Roman", serif;
    line-height: 1.7;
  }

  :global(code),
  :global(pre) {
    font-family: "SF Mono", Consolas, "Liberation Mono", Menlo, monospace;
  }

  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 1rem 2rem;
    flex-shrink: 0;
  }

  .logo {
    color: #e8e4df;
    text-decoration: none;
    font-size: 1rem;
  }

  .logo:hover {
    color: #fff;
  }

  .header-right {
    display: flex;
    align-items: center;
    gap: 1.5rem;
  }

  .header-right a {
    color: #6a6560;
    text-decoration: none;
    display: flex;
    align-items: center;
  }

  .header-right a:hover {
    color: #a8a4a0;
  }

  main {
    display: flex;
    flex: 1;
    overflow: hidden;
    gap: 2rem;
    padding: 0 2rem 2rem;
  }

  .left-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-width: 0;
  }

  .vim-toggle {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.8rem;
    cursor: pointer;
    user-select: none;
    color: #6a6560;
  }

  .vim-toggle:hover {
    color: #a8a4a0;
  }

  .vim-toggle input {
    cursor: pointer;
  }

  .header-btn {
    background: rgba(255, 255, 255, 0.06);
    border: none;
    color: #a8a4a0;
    padding: 0.4rem 1rem;
    font-family: Georgia, "Times New Roman", serif;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .header-btn:hover:not(:disabled) {
    color: #e8e4df;
    background: rgba(255, 255, 255, 0.1);
  }

  .header-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .right-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-width: 0;
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
    header {
      padding: 0.75rem 1rem;
    }

    main {
      flex-direction: column;
      padding: 0 1rem 1rem;
      gap: 1rem;
    }

    .left-panel {
      height: 50%;
    }

    .right-panel {
      height: 50%;
    }
  }
</style>

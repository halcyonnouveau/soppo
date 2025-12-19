<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { lessons } from "$lib/lessons";

  let { children } = $props();

  // Get current lesson index
  let currentIndex = $derived(
    lessons.findIndex((l) => l.slug === page.params.lesson),
  );
  let currentLesson = $derived(lessons[currentIndex]);
  let prevLesson = $derived(
    currentIndex > 0 ? lessons[currentIndex - 1] : null,
  );
  let nextLesson = $derived(
    currentIndex < lessons.length - 1 ? lessons[currentIndex + 1] : null,
  );
  let isContentsPage = $derived(page.url.pathname === "/contents");

  function goToLesson(index: number) {
    if (index >= 0 && index < lessons.length) {
      goto(`/${lessons[index].slug}`);
    }
  }
</script>

<svelte:head>
  <title
    >{currentLesson
      ? `${currentLesson.title} · Soppo Language Tour`
      : "Soppo Language Tour"}</title
  >
  <meta name="description" content="Learn Soppo through interactive examples" />
</svelte:head>

<div class="app">
  <header>
    <a href="/contents" class="logo">Soppo Language Tour</a>
    <div class="header-links">
      <a href="https://soppolang.dev">soppolang.dev</a>
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
    {@render children()}
  </main>

  {#if currentIndex >= 0 && !isContentsPage}
    <nav class="bottom-nav">
      <button
        class="nav-btn prev"
        onclick={() => prevLesson && goto(`/${prevLesson.slug}`)}
        disabled={!prevLesson}
      >
        <span class="arrow">←</span>
        <span class="label">Prev</span>
      </button>

      <div class="progress">
        <div class="dots">
          {#each lessons as lesson, i}
            <button
              class="dot"
              class:active={i === currentIndex}
              class:visited={i < currentIndex}
              onclick={() => goToLesson(i)}
              aria-label={lesson.title}
            ></button>
          {/each}
        </div>
        <a href="/contents" class="position">Table of Contents</a>
      </div>

      <button
        class="nav-btn next"
        onclick={() => nextLesson && goto(`/${nextLesson.slug}`)}
        disabled={!nextLesson}
      >
        <span class="label">Next</span>
        <span class="arrow">→</span>
      </button>
    </nav>
  {/if}
</div>

<style>
  :global(*) {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
  }

  :global(html),
  :global(body) {
    min-height: 100%;
  }

  @media (min-width: 901px) {
    :global(html),
    :global(body) {
      height: 100%;
      overflow: hidden;
    }
  }

  :global(html) {
    font-size: 18px;
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
    min-height: 100%;
  }

  @media (min-width: 901px) {
    .app {
      height: 100%;
      overflow: hidden;
      position: fixed;
      inset: 0;
    }
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

  .header-links {
    display: flex;
    align-items: center;
    gap: 1.5rem;
  }

  .header-links a {
    color: #6a6560;
    text-decoration: none;
    font-size: 0.9rem;
    display: flex;
    align-items: center;
  }

  .header-links a:hover {
    color: #a8a4a0;
  }

  main {
    flex: 1;
    overflow: hidden;
  }

  .bottom-nav {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    padding: 1rem 2rem;
    flex-shrink: 0;
  }

  .nav-btn {
    display: flex;
    align-items: center;
    align-self: center;
    gap: 0.5rem;
    background: none;
    border: none;
    color: #6a6560;
    font-family: Georgia, "Times New Roman", serif;
    font-size: 0.9rem;
    cursor: pointer;
    padding: 0;
    min-width: 80px;
    height: fit-content;
  }

  .nav-btn:hover:not(:disabled) {
    color: #e8e4df;
  }

  .nav-btn:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }

  .nav-btn.prev {
    justify-self: start;
  }

  .nav-btn.next {
    justify-self: end;
  }

  .arrow {
    font-size: 1.1rem;
    transform: translateY(-2px);
  }

  .progress {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
  }

  .dots {
    display: flex;
    gap: 0.4rem;
  }

  .dot {
    width: 8px;
    height: 8px;
    border: none;
    background: #3a3a3a;
    cursor: pointer;
    padding: 0;
    transition:
      background 0.15s,
      transform 0.15s;
  }

  .dot:hover {
    background: #5a5a5a;
    transform: scale(1.2);
  }

  .dot.visited {
    background: #4a4a4a;
  }

  .dot.active {
    background: #e8e4df;
  }

  .position {
    color: #6a6560;
    font-size: 0.8rem;
    text-decoration: none;
  }

  .position:hover {
    color: #a8a4a0;
  }

  @media (max-width: 768px) {
    header {
      padding: 0.75rem 1rem;
    }

    .bottom-nav {
      padding: 0.75rem 1rem;
    }

    .nav-btn .label {
      display: none;
    }

    .nav-btn {
      min-width: 40px;
    }

    .dots {
      gap: 0.3rem;
    }

    .dot {
      width: 6px;
      height: 6px;
    }

    .position {
      font-size: 0.75rem;
    }
  }
</style>

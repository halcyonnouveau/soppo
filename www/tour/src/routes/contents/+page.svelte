<script lang="ts">
  import { lessons } from "$lib/lessons";
</script>

<div class="contents">
  <h1>Tour of Soppo</h1>
  <p class="intro">
    Learn Soppo through interactive examples. Each lesson introduces a feature
    with editable code that runs in your browser.
  </p>

  <nav class="toc">
    {#each lessons as lesson, i}
      <a href="/{lesson.slug}" class="toc-link">
        <span class="num">{i + 1}</span>
        <span class="title">{lesson.title}</span>
      </a>
    {/each}
  </nav>
</div>

<style>
  .contents {
    max-width: 500px;
    margin: 0 auto;
    padding: 2rem;
  }

  h1 {
    font-size: 1.5rem;
    font-weight: normal;
    margin-bottom: 1rem;
    color: #e8e4df;
  }

  .intro {
    color: #a8a4a0;
    margin-bottom: 2rem;
    font-size: 0.95rem;
  }

  .toc {
    display: flex;
    flex-direction: column;
  }

  .toc-link {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.2rem 0;
    color: #8a8580;
    text-decoration: none;
    transition: color 0.15s;
  }

  .toc-link:hover {
    color: #e8e4df;
  }

  .num {
    color: #5a5550;
    font-size: 0.85rem;
    min-width: 1.5rem;
  }

  .title {
    font-size: 0.95rem;
  }

  @media (max-width: 600px) {
    .contents {
      padding: 1rem;
    }
  }
</style>

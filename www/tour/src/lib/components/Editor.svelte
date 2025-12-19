<script lang="ts">
  import { onMount } from "svelte";
  import {
    EditorView,
    keymap,
    highlightSpecialChars,
    drawSelection,
    dropCursor,
    rectangularSelection,
    crosshairCursor,
  } from "@codemirror/view";
  import { EditorState, Compartment } from "@codemirror/state";
  import {
    indentWithTab,
    history,
    defaultKeymap,
    historyKeymap,
  } from "@codemirror/commands";
  import {
    indentUnit,
    indentOnInput,
    bracketMatching,
    foldKeymap,
  } from "@codemirror/language";
  import { highlightSelectionMatches, searchKeymap } from "@codemirror/search";
  import { go } from "@codemirror/lang-go";
  import { vim } from "@replit/codemirror-vim";
  import { ayuDark } from "$lib/theme";

  interface Props {
    value: string;
    readonly?: boolean;
    vimMode?: boolean;
    onchange?: (value: string) => void;
    onrun?: () => void;
  }

  let {
    value = $bindable(),
    readonly = false,
    vimMode = false,
    onchange,
    onrun,
  }: Props = $props();

  let container: HTMLDivElement;
  let view: EditorView;
  let vimCompartment = new Compartment();

  onMount(() => {
    const extensions = [
      highlightSpecialChars(),
      history(),
      drawSelection(),
      dropCursor(),
      indentOnInput(),
      bracketMatching(),
      rectangularSelection(),
      crosshairCursor(),
      highlightSelectionMatches(),
      keymap.of([
        indentWithTab,
        ...defaultKeymap,
        ...historyKeymap,
        ...foldKeymap,
        ...searchKeymap,
        {
          key: "Ctrl-Enter",
          mac: "Cmd-Enter",
          run: () => {
            onrun?.();
            return true;
          },
        },
      ]),
      go(),
      ayuDark,
      indentUnit.of("\t"),
      EditorState.tabSize.of(4),
      vimCompartment.of(vimMode ? vim() : []),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) {
          value = update.state.doc.toString();
          onchange?.(value);
        }
      }),
    ];

    if (readonly) {
      extensions.push(EditorState.readOnly.of(true));
    }

    view = new EditorView({
      state: EditorState.create({
        doc: value,
        extensions,
      }),
      parent: container,
    });

    return () => {
      view.destroy();
    };
  });

  $effect(() => {
    if (view) {
      view.dispatch({
        effects: vimCompartment.reconfigure(vimMode ? vim() : []),
      });
    }
  });

  $effect(() => {
    if (view && value !== view.state.doc.toString()) {
      view.dispatch({
        changes: {
          from: 0,
          to: view.state.doc.length,
          insert: value,
        },
      });
    }
  });
</script>

<div bind:this={container} class="editor"></div>

<style>
  .editor {
    height: 100%;
    overflow: auto;
  }

  .editor :global(.cm-editor) {
    height: 100%;
  }

  .editor :global(.cm-scroller) {
    overflow: auto;
  }

  .editor :global(.cm-content) {
    font-size: 13px;
  }
</style>

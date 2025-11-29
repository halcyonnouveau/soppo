<script lang="ts">
  import { onMount } from "svelte";
  import { EditorView, keymap } from "@codemirror/view";
  import { EditorState, Compartment } from "@codemirror/state";
  import { indentWithTab } from "@codemirror/commands";
  import { indentUnit } from "@codemirror/language";
  import { basicSetup } from "codemirror";
  import { go } from "@codemirror/lang-go";
  import { vim } from "@replit/codemirror-vim";
  import { oneDark } from "@codemirror/theme-one-dark";

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
      keymap.of([
        indentWithTab,
        {
          key: "Ctrl-Enter",
          mac: "Cmd-Enter",
          run: () => {
            onrun?.();
            return true;
          },
        },
      ]),
      basicSetup,
      go(),
      oneDark,
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
</style>

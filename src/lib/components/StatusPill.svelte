<script lang="ts">
  import type { TaskStatus } from "$lib/types";
  let { status }: { status: TaskStatus } = $props();
</script>

<span
  class:running={status === "running"}
  class:error={status === "error"}
  class:done={status === "completed"}
  class="status"><i></i>{status}</span
>

<style>
  .status {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font: calc(11px * var(--interface-font-ratio, 1)) var(--mono);
    color: var(--muted);
    text-transform: capitalize;
  }
  i {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #aaa092;
  }
  .running i {
    background: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .error i {
    background: #c44c4c;
  }
  .done i {
    background: #6f9278;
  }
</style>

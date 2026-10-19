<script>
  let { size = 18, class: className = '' } = $props();
</script>

<span class={className} aria-hidden="true" style={`width:${size}px;height:${size}px;flex:none;display:inline-block`}></span>

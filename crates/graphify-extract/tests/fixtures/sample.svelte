<script>
  import { onMount } from 'svelte';
  import Button from './Button.svelte';

  export let title = 'Hello';
  let count = 0;

  function increment() {
    count += 1;
  }

  onMount(() => {
    console.log('mounted');
  });
</script>

<style>
  h1 { color: blue; }
</style>

<h1>{title}</h1>
<Button onClick={increment}>Click {count}</Button>

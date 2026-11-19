<script>
  import { iconSvg } from './icons.js';

  /** @type {keyof typeof import('./icons.js').ICONS} */
  export let name;
</script>

{@html iconSvg(name, $$props.class)}

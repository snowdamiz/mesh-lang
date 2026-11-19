<script setup lang="ts">
import { usePrevNext } from '../../composables/usePrevNext'

const pager = usePrevNext()
</script>

<template>
  <nav v-if="pager.prev?.link || pager.next?.link" class="pager" aria-label="Previous and next">
    <a v-if="pager.prev?.link" :href="pager.prev.link"><small>Previous</small><b>{{ pager.prev.text }}</b></a>
    <a v-if="pager.next?.link" class="next" :href="pager.next.link"><small>Next</small><b>{{ pager.next.text }}</b></a>
  </nav>
</template>

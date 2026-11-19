<script setup lang="ts">
import { computed, ref, useId } from 'vue'
import type { LandingData } from './landing.data.mjs'
import { useRovingTabs } from '../../composables/useRovingTabs'

const props = defineProps<{ toolchain: LandingData['toolchain'] }>()
const id = useId()
const selected = ref(0)
const command = computed(() => props.toolchain.commands[selected.value])
const onTabKey = useRovingTabs((i) => { selected.value = i })
</script>

<template>
  <div class="toolbox">
    <div :id="`${id}-panel`" role="tabpanel" :aria-labelledby="`${id}-${command.id}`" aria-live="polite">
      <p class="prompt"><span class="pr" aria-hidden="true">$ </span>{{ command.command }}</p>
      <p class="prompt-gloss">{{ command.gloss }}</p>
    </div>
    <div class="toolbox-foot">
      <div class="tool-pills" role="tablist" aria-label="Commands" @keydown="onTabKey">
        <button
          v-for="(c, i) in toolchain.commands"
          :id="`${id}-${c.id}`"
          :key="c.id"
          type="button"
          role="tab"
          :aria-selected="i === selected"
          :aria-controls="`${id}-panel`"
          :tabindex="i === selected ? 0 : -1"
          @click="selected = i"
        >{{ c.id }}</button>
      </div>
      <p class="toolbox-note">{{ toolchain.note }}</p>
    </div>
  </div>
</template>

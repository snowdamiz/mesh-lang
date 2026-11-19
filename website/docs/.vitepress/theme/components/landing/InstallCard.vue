<script setup lang="ts">
import { computed, onMounted, ref, useId } from 'vue'
import CopyButton from '../CopyButton.vue'
import { useRovingTabs } from '../../composables/useRovingTabs'
import type { InstallOption } from './landing.data.mjs'

const props = defineProps<{ options: InstallOption[]; note?: boolean; firstRun?: string[] }>()

const id = useId()
const selected = ref(props.options[0].id)
const option = computed(() => props.options.find((o) => o.id === selected.value)!)
const onTabKey = useRovingTabs((i) => { selected.value = props.options[i].id })

onMounted(() => {
  if (/Win/i.test(navigator.platform)) selected.value = 'win'
})
</script>

<template>
  <div class="install">
    <div class="seg" role="tablist" aria-label="Operating system" @keydown="onTabKey">
      <button
        v-for="o in options"
        :id="`${id}-${o.id}`"
        :key="o.id"
        type="button"
        role="tab"
        :aria-selected="o.id === selected"
        :aria-controls="`${id}-panel`"
        :tabindex="o.id === selected ? 0 : -1"
        @click="selected = o.id"
      >{{ o.label }}</button>
    </div>
    <div :id="`${id}-panel`" class="cmd" role="tabpanel" :aria-labelledby="`${id}-${selected}`">
      <code><span class="pr" aria-hidden="true">$ </span>{{ option.command }}</code>
      <CopyButton :text="option.command" label="Copy" action="Copy install command" />
    </div>
    <p v-if="note" class="install-note">{{ option.note }}</p>
    <div v-if="firstRun" class="install-then">
      <div class="install-then-head">
        <p>Then create a project</p>
        <CopyButton :text="firstRun.join('\n')" action="Copy project commands" />
      </div>
      <pre class="steps"><template v-for="(line, i) in firstRun" :key="line"><span class="pr" aria-hidden="true">$ </span>{{ line }}{{ i < firstRun.length - 1 ? '\n' : '' }}</template></pre>
    </div>
  </div>
</template>

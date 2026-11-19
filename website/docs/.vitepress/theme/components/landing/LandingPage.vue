<script setup lang="ts">
import { data } from './landing.data.mjs'
import morse from '../../assets/morse-avatar.svg?raw'
import { vReveal } from '../../composables/motion'
import InstallCard from './InstallCard.vue'
import ProductDemo from './ProductDemo.vue'
import FeatureBento from './FeatureBento.vue'
import ClusterFigure from './ClusterFigure.vue'
import ToolchainTabs from './ToolchainTabs.vue'
import StdlibEditor from './StdlibEditor.vue'

const { hero, install, demo, features, art, distribution, toolchain, stdlib, quote, close } = data
</script>

<template>
  <main id="main">
    <section class="hero wrap" aria-labelledby="h-hero">
      <div class="hero-copy">
        <h1 id="h-hero" class="display"><template v-for="(line, i) in hero.title" :key="line"><br v-if="i > 0" />{{ line }}</template></h1>
        <p class="lead">{{ hero.lead }}</p>
        <div class="hero-actions"><a class="btn btn-ink btn-lg" :href="hero.cta.href">{{ hero.cta.text }}</a></div>
      </div>
      <InstallCard :options="install" note :first-run="hero.firstRun" />
    </section>

    <ProductDemo :demo="demo" />

    <section id="features" class="section" aria-labelledby="h-features">
      <div class="wrap">
        <h2 id="h-features" v-reveal class="h2 section-title">{{ features.title[0] }} <span class="grey">{{ features.title[1] }}</span></h2>
        <FeatureBento :tiles="features.tiles" :art="art" />
      </div>
    </section>

    <section id="distribution" class="section" aria-labelledby="h-dist">
      <div class="wrap">
        <div v-reveal class="section-head">
          <h2 id="h-dist" class="h2">{{ distribution.title[0] }} <span class="grey">{{ distribution.title[1] }}</span></h2>
          <div class="lead">
            <p style="margin: 0" v-html="distribution.lead" />
            <a class="link-arrow" :href="distribution.link.href">{{ distribution.link.text }} <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><path d="M3 8h10M9 4l4 4-4 4" /></svg></a>
          </div>
        </div>
        <ClusterFigure :figure="distribution.figure" />
        <div v-reveal class="codecard on-night"><pre class="code"><code v-html="distribution.code" /></pre></div>
      </div>
    </section>

    <section id="toolchain" class="section" aria-labelledby="h-tool">
      <div class="wrap">
        <h2 id="h-tool" v-reveal class="h2 section-title">{{ toolchain.title[0] }} <span class="grey">{{ toolchain.title[1] }}</span></h2>
        <ToolchainTabs v-reveal :toolchain="toolchain" />
      </div>
    </section>

    <section id="stdlib" class="section" aria-labelledby="h-std">
      <div class="wrap">
        <h2 id="h-std" v-reveal class="h2 section-title">{{ stdlib.title[0] }} <span class="grey">{{ stdlib.title[1] }}</span></h2>
        <StdlibEditor v-reveal :stdlib="stdlib" />
      </div>
    </section>

    <section id="origin" class="section" aria-label="Origin">
      <figure v-reveal class="quote wrap">
        <blockquote><p>{{ quote.text }}</p></blockquote>
        <figcaption>
          <span class="avatar" aria-hidden="true" v-html="morse" />
          <span><b>{{ quote.author }}</b>{{ quote.role }}</span>
        </figcaption>
      </figure>
    </section>

    <section id="install" class="section" aria-labelledby="h-install">
      <div v-reveal class="wrap close">
        <h2 id="h-install" class="display">{{ close.title }}</h2>
        <InstallCard :options="install" />
      </div>
    </section>
  </main>
</template>

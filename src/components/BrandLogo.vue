<script setup lang="ts">
withDefaults(defineProps<{ size?: number }>(), { size: 46 });
</script>

<template>
  <svg
    class="zhihua-logo"
    :width="size"
    :height="size"
    viewBox="0 0 52 52"
    fill="none"
    aria-hidden="true"
  >
    <defs>
      <linearGradient id="zhihua-logo-gradient" x1="5" y1="3" x2="47" y2="49" gradientUnits="userSpaceOnUse">
        <stop stop-color="#075FEA" />
        <stop offset=".55" stop-color="#278AFF" />
        <stop offset="1" stop-color="#7055E9" />
      </linearGradient>
      <linearGradient id="zhihua-brush-gradient" x1="10" y1="39" x2="41" y2="15" gradientUnits="userSpaceOnUse">
        <stop stop-color="#A9FFE8" />
        <stop offset="1" stop-color="#FFFFFF" />
      </linearGradient>
    </defs>
    <rect x="2" y="2" width="48" height="48" rx="14" fill="url(#zhihua-logo-gradient)" />
    <rect x="3" y="3" width="46" height="46" rx="13" stroke="white" stroke-opacity=".2" stroke-width="2" />
    <rect x="11" y="15" width="8" height="21" rx="3" fill="white" fill-opacity=".26" stroke="white" stroke-opacity=".84" stroke-width="2" />
    <rect x="22" y="12" width="8" height="25" rx="3" fill="white" fill-opacity=".38" stroke="white" stroke-width="2" />
    <rect x="33" y="17" width="8" height="18" rx="3" fill="white" fill-opacity=".25" stroke="white" stroke-opacity=".86" stroke-width="2" />
    <path d="M10.5 39C16.7 34.8 20.3 30.6 24.2 33.4C29.7 37.3 31.6 26.6 41.5 23" stroke="url(#zhihua-brush-gradient)" stroke-width="4.4" stroke-linecap="round" />
    <path d="M39.5 9L40.65 11.85L43.5 13L40.65 14.15L39.5 17L38.35 14.15L35.5 13L38.35 11.85L39.5 9Z" fill="white" />
  </svg>
</template>

<style scoped>
.zhihua-logo{display:block;flex:0 0 auto;filter:drop-shadow(0 7px 12px rgba(30,89,210,.24))}
</style>

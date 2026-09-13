<script setup lang="ts">
withDefaults(defineProps<{ size?: number }>(), { size: 34 });
</script>

<template>
  <svg
    class="zhihua-logo"
    :width="size"
    :height="size"
    viewBox="0 0 48 48"
    fill="none"
    aria-hidden="true"
  >
    <defs>
      <linearGradient id="zhihua-logo-gradient" x1="5" y1="4" x2="43" y2="44" gradientUnits="userSpaceOnUse">
        <stop stop-color="#1687FF" />
        <stop offset="1" stop-color="#7257E8" />
      </linearGradient>
    </defs>
    <rect x="2" y="2" width="44" height="44" rx="13" fill="url(#zhihua-logo-gradient)" />
    <path d="M11.5 15.5C11.5 13.29 13.29 11.5 15.5 11.5H31.5C33.71 11.5 35.5 13.29 35.5 15.5V26.5C35.5 28.71 33.71 30.5 31.5 30.5H15.5C13.29 30.5 11.5 28.71 11.5 26.5V15.5Z" fill="white" fill-opacity=".17" stroke="white" stroke-width="2.4" />
    <path d="M17 20.5L21.2 24.7L27.4 17.8L35.2 26.2" stroke="white" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" />
    <circle cx="29.3" cy="17" r="2.1" fill="#9CF2DF" />
    <path d="M15 36C21.4 32.4 28.4 32.7 35 36" stroke="#9CF2DF" stroke-width="3" stroke-linecap="round" />
    <path d="M37.5 8.7L38.5 11.1L40.9 12.1L38.5 13.1L37.5 15.5L36.5 13.1L34.1 12.1L36.5 11.1L37.5 8.7Z" fill="white" />
  </svg>
</template>

<style scoped>
.zhihua-logo{display:block;flex:0 0 auto;filter:drop-shadow(0 5px 9px rgba(40,91,210,.2))}
</style>

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
      <linearGradient id="zhihua-mark" x1="6" y1="5" x2="46" y2="47" gradientUnits="userSpaceOnUse">
        <stop stop-color="#086BFA" />
        <stop offset="1" stop-color="#594DE8" />
      </linearGradient>
      <linearGradient id="zhihua-flow" x1="13" y1="34" x2="40" y2="23" gradientUnits="userSpaceOnUse">
        <stop stop-color="#A7F3FF" />
        <stop offset="1" stop-color="#FFFFFF" />
      </linearGradient>
    </defs>
    <rect x="2" y="2" width="48" height="48" rx="13" fill="url(#zhihua-mark)" />
    <rect x="3" y="3" width="46" height="46" rx="12" stroke="white" stroke-opacity=".22" stroke-width="2" />
    <rect x="9" y="12" width="34" height="28" rx="7" fill="white" fill-opacity=".14" stroke="white" stroke-width="2.2" />
    <path d="M14 12V40M38 12V40" stroke="white" stroke-opacity=".74" stroke-width="2" />
    <path d="M14 18H10M14 25H10M14 32H10M42 18H38M42 25H38M42 32H38" stroke="white" stroke-width="2" stroke-linecap="round" />
    <path d="M17.5 32.5C21 24.3 25 19.5 28.7 22.2C31.8 24.5 30.2 29 35.5 18.5" stroke="url(#zhihua-flow)" stroke-width="3.5" stroke-linecap="round" stroke-linejoin="round" />
    <path d="M32.5 14.2L33.6 16.7L36.2 17.8L33.6 18.9L32.5 21.4L31.4 18.9L28.8 17.8L31.4 16.7L32.5 14.2Z" fill="white" />
  </svg>
</template>

<style scoped>
.zhihua-logo{display:block;flex:0 0 auto;filter:drop-shadow(0 7px 12px rgba(30,89,210,.24))}
</style>

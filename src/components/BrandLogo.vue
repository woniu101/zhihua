<script setup lang="ts">
withDefaults(defineProps<{ size?: number }>(), { size: 42 });
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
      <linearGradient id="zhihua-flow" x1="14" y1="33" x2="39" y2="16" gradientUnits="userSpaceOnUse">
        <stop stop-color="#B8FFE9" />
        <stop offset="1" stop-color="#FFFFFF" />
      </linearGradient>
    </defs>
    <rect x="2" y="2" width="48" height="48" rx="13" fill="url(#zhihua-mark)" />
    <rect x="3" y="3" width="46" height="46" rx="12" stroke="white" stroke-opacity=".22" stroke-width="2" />
    <rect x="10" y="10" width="32" height="31" rx="7.5" fill="white" fill-opacity=".13" stroke="white" stroke-width="2.2" />
    <path d="M15 32C19.8 28.6 21.7 30.2 26.8 23.4C30.6 18.4 33.2 21.2 38 15.8" stroke="url(#zhihua-flow)" stroke-width="3.2" stroke-linecap="round" stroke-linejoin="round" />
    <circle cx="15" cy="32" r="2.6" fill="white" />
    <circle cx="27" cy="23" r="2.6" fill="white" />
    <circle cx="38" cy="16" r="2.6" fill="white" />
    <path d="M15 37H37" stroke="white" stroke-opacity=".55" stroke-width="2" stroke-linecap="round" />
    <path d="M18 35.5V38.5M26 35.5V38.5M34 35.5V38.5" stroke="white" stroke-opacity=".8" stroke-width="1.8" stroke-linecap="round" />
  </svg>
</template>

<style scoped>
.zhihua-logo{display:block;flex:0 0 auto;filter:drop-shadow(0 7px 12px rgba(30,89,210,.24))}
</style>

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
        <stop stop-color="#B8FFE9" />
        <stop offset="1" stop-color="#FFFFFF" />
      </linearGradient>
    </defs>
    <rect x="2" y="2" width="48" height="48" rx="13" fill="url(#zhihua-mark)" />
    <rect x="3" y="3" width="46" height="46" rx="12" stroke="white" stroke-opacity=".22" stroke-width="2" />
    <path d="M11 13.5C11 11.57 12.57 10 14.5 10H25V38H14.5C12.57 38 11 36.43 11 34.5V13.5Z" fill="white" fill-opacity=".18" stroke="white" stroke-width="2" />
    <path d="M15 16H21M15 21H21M15 26H19" stroke="white" stroke-width="2.2" stroke-linecap="round" />
    <rect x="27.5" y="12" width="13.5" height="7.5" rx="2.2" fill="white" fill-opacity=".9" />
    <rect x="27.5" y="22.25" width="13.5" height="7.5" rx="2.2" fill="white" fill-opacity=".68" />
    <rect x="27.5" y="32.5" width="13.5" height="7.5" rx="2.2" fill="white" fill-opacity=".44" />
    <path d="M20.5 33.5C24 31 25.2 27 29 26C32.1 25.2 34 27.2 39.5 23.5" stroke="url(#zhihua-flow)" stroke-width="3.2" stroke-linecap="round" />
  </svg>
</template>

<style scoped>
.zhihua-logo{display:block;flex:0 0 auto;filter:drop-shadow(0 7px 12px rgba(30,89,210,.24))}
</style>

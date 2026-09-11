import { readonly, ref } from "vue";

export type ThemePreference = "light" | "dark" | "system";
export type EffectiveTheme = Exclude<ThemePreference, "system">;

const STORAGE_KEY = "zhihua.appearance.theme.v1";
const saved = localStorage.getItem(STORAGE_KEY);
const initialPreference: ThemePreference = saved === "light" || saved === "dark" || saved === "system"
  ? saved
  : "light";

const preference = ref<ThemePreference>(initialPreference);
const effective = ref<EffectiveTheme>("light");
let mediaQuery: MediaQueryList | undefined;
let initialized = false;

function resolveTheme(value: ThemePreference): EffectiveTheme {
  if (value !== "system") return value;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function applyTheme() {
  effective.value = resolveTheme(preference.value);
  document.documentElement.dataset.theme = effective.value;
  document.documentElement.dataset.themePreference = preference.value;
}

export function initializeAppearance() {
  if (initialized) return;
  initialized = true;
  mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
  mediaQuery.addEventListener("change", () => {
    if (preference.value === "system") applyTheme();
  });
  applyTheme();
}

export function setThemePreference(value: ThemePreference) {
  preference.value = value;
  localStorage.setItem(STORAGE_KEY, value);
  applyTheme();
}

export const themePreference = readonly(preference);
export const effectiveTheme = readonly(effective);

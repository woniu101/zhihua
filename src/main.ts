import { createApp } from "vue";
import { createRouter, createWebHashHistory } from "vue-router";
import App from "./App.vue";
import AssetsView from "./views/AssetsView.vue";
import ExportView from "./views/ExportView.vue";
import ProjectsView from "./views/ProjectsView.vue";
import SettingsView from "./views/SettingsView.vue";
import SourcesView from "./views/SourcesView.vue";
import StoryboardView from "./views/StoryboardView.vue";
import { initializeAppearance } from "./services/appearance";
import { activeProjectId } from "./services/storyboardRepository";
import "./styles.css";

initializeAppearance();

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", redirect: "/projects" },
    { path: "/projects", component: ProjectsView },
    { path: "/sources", component: SourcesView, meta: { requiresProject: true } },
    { path: "/storyboard", component: StoryboardView, meta: { requiresProject: true } },
    { path: "/assets", component: AssetsView, meta: { requiresProject: true } },
    { path: "/export", component: ExportView, meta: { requiresProject: true } },
    { path: "/settings", component: SettingsView },
  ],
});

router.beforeEach((to) => {
  if (to.meta.requiresProject && !activeProjectId()) return "/projects";
  return true;
});

createApp(App).use(router).mount("#app");

import { createApp } from "vue";
import { createRouter, createWebHashHistory } from "vue-router";
import App from "./App.vue";
import AssetsView from "./views/AssetsView.vue";
import ExportView from "./views/ExportView.vue";
import ProjectsView from "./views/ProjectsView.vue";
import SettingsView from "./views/SettingsView.vue";
import SourcesView from "./views/SourcesView.vue";
import StoryboardView from "./views/StoryboardView.vue";
import "./styles.css";

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", redirect: "/projects" },
    { path: "/projects", component: ProjectsView },
    { path: "/sources", component: SourcesView },
    { path: "/storyboard", component: StoryboardView },
    { path: "/assets", component: AssetsView },
    { path: "/export", component: ExportView },
    { path: "/settings", component: SettingsView },
  ],
});

createApp(App).use(router).mount("#app");

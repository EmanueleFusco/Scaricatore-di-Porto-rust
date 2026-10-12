import { mount } from "svelte";
import "./lib/styles/app.css";
import App from "./App.svelte";

const target = document.getElementById("app");
if (!target) {
  // Cannot happen: index.html ships the element. Fail loudly rather than
  // rendering nothing.
  throw new Error("#app is missing from index.html");
}

export default mount(App, { target });
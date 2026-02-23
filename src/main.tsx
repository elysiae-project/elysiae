import { render } from "preact";
import App from "./App";

window.addEventListener("online", () => {});

window.addEventListener("offline", () => {});

render(<App />, document.getElementById("root")!);

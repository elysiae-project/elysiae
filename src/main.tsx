/** biome-ignore-all lint/style/noNonNullAssertion: This is just how preact works.... */
import { render } from "preact";
import App from "./App";

render(<App />, document.getElementById("root")!);

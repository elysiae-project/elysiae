import { getCurrentWindow } from "@tauri-apps/api/window";

export function closeApp() {
	getCurrentWindow().close();
}

export function minimizeApp() {
	getCurrentWindow().minimize();
}

import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";
import preact from "@preact/preset-vite";

const host = process.env.TAURI_DEV_HOST;
export default defineConfig(async () => ({
	plugins: [preact(), tailwindcss()],
	clearScreen: false,
	server: {
		port: 1420,
		strictPort: true,
		host: host || false,
		hmr: host
			? {
					protocol: "ws",
					host,
					port: 1421,
				}
			: undefined,
		watch: {
			ignored: [
				"**/.flatpak-builder/**",
				"**/build/**",
				"**/dist/**",
				"**/repo/**",
				"**/src-tauri/**",
			],
		},
	},
}));

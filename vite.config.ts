import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";
import preact from "@preact/preset-vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
export default defineConfig(async () => ({
	plugins: [preact(), tailwindcss()],
	resolve: {
		alias: {
			react: "preact/compat",
			"react-dom": "preact/compat",
			"react/jsx-runtime": "preact/jsx-runtime",
		},
	},
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
			ignored: ["**/build/**", "**/src-tauri/**"],
		},
	},
}));

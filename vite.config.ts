import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";
import preact from "@preact/preset-vite";
import { readFileSync } from "node:fs";

const isWSL = () => {
	if (process.env.WSL_DISTRO_NAME || process.env.WSL_INTEROP) {
		return true;
	}

	if (process.platform === "linux") {
		try {
			const version = readFileSync("/proc/version", "utf8");
			return /microsoft|wsl/i.test(version);
		} catch {
			return false;
		}
	}

	return false;
};

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
			usePolling: isWSL(),
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

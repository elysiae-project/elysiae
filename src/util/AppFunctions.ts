import { getCurrentWindow } from "@tauri-apps/api/window";
import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import { fetch } from "@tauri-apps/plugin-http";
import { error, info } from "@tauri-apps/plugin-log";
import { download } from "@tauri-apps/plugin-upload";

export const closeApp = (): void => {
	getCurrentWindow().close();
};

export const minimizeApp = (): void => {
	getCurrentWindow().minimize();
};

export const getActiveGameCode = (): "bh" | "ys" | "sr" | "nap" => {
	const { game, setGame } = useGame();

	switch (game) {
		case Variants.BH:
			return "bh";
		case Variants.YS:
			return "ys";
		case Variants.SR:
			return "sr";
		case Variants.NAP:
			return "nap";
	}
};

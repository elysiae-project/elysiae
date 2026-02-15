import { getCurrentWindow } from "@tauri-apps/api/window";
import { useGame } from "../hooks/useGame";
import { Variants } from "../types";

export const closeApp = () => {
	getCurrentWindow().close();
};

export const minimizeApp = () => {
	getCurrentWindow().minimize();
};

export const getActiveGameCode = () => {
	switch (useGame()) {
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

import { getCurrentWindow, Window } from "@tauri-apps/api/window";

import { useGame } from "../hooks/useGame.ts";
import { Variants } from "../types";

import BhTitlebar from "./bh/Titlebar.tsx";
import NapTitlebar from "./nap/Titlebar.tsx";
import SrTitlebar from "./sr/Titlebar.tsx";
import YsTitlebar from "./ys/Titlebar.tsx";
import { useApi } from "../hooks/useApi.ts";

const appWindow: Window = getCurrentWindow();

const closeWindow = () => {
	appWindow.close();
};

const toggleMaximize = () => {
	appWindow.toggleMaximize();
};

const minimize = () => {
	appWindow.minimize();
};

export default function Titlebar() {
	const game = useGame();

	const Titlebar = {
		[Variants.BH]: BhTitlebar,
		[Variants.YS]: YsTitlebar,
		[Variants.SR]: SrTitlebar,
		[Variants.NAP]: NapTitlebar,
	}[game];
	return (
		<Titlebar
			onClose={closeWindow}
			onToggleMaximize={toggleMaximize}
			onMinimize={minimize}
		/>
	);
}

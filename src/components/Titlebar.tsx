import { cva } from "class-variance-authority";
import { useGame } from "../hooks/useGame.ts";
import { Variants } from "../types";

import BhTitlebar from "./bh/Titlebar.tsx";
import NapTitlebar from "./nap/Titlebar.tsx";
import SrTitlebar from "./sr/Titlebar.tsx";
import YsTitlebar from "./ys/Titlebar.tsx";
import { closeApp } from "../util/AppFunctions.ts";

function TitlebarButtons() {
	const buttonStyles = cva("h-10 w-10 flex items-center justify-center", {
		variants: {
			intent: {
				[Variants.BH]: "",
				[Variants.YS]:
					"border-3 p-0.5 border-[#888d8e] bg-[#ece5d8] hover:border-transparent hover:drop-shadow-xs hover:drop-shadow[#fdfdfeAA] rounded-full",
				[Variants.SR]: "",
				[Variants.NAP]: "",
			},
		},
	});

	const gamePath = (() => {
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
	})();
	const assetPath = `src/assets/icon/${gamePath}`;

	return (
		<div class={buttonStyles({ intent: useGame() })} onClick={() => closeApp()}>
			<img src={`${assetPath}/close.svg`} width={18} height={18}  />
		</div>
	);
}

export default function Titlebar() {
	const game = useGame();

	const Titlebar = {
		[Variants.BH]: BhTitlebar,
		[Variants.YS]: YsTitlebar,
		[Variants.SR]: SrTitlebar,
		[Variants.NAP]: NapTitlebar,
	}[game];
	return (
		<div style={{ zIndex: 1001, color: "white" }}>
			<Titlebar>
				<TitlebarButtons />
			</Titlebar>
		</div>
	);
}

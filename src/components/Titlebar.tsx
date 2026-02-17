import { cva } from "class-variance-authority";
import { useGame } from "../hooks/useGame.ts";
import { Variants } from "../types";
import { closeApp, getActiveGameCode } from "../util/AppFunctions.ts";
import { useState } from "preact/hooks";

const titlebarStyles = cva(
	"h-15 min-w-full p-1 transition-all duration-175 flex flex-row justify-between items-center px-5 py-1.5",
	{
		variants: {
			game: {
				// Declare fonts because the titlebar is declared outside the actual app content. Can be easily fixed if I wasn't lazy
				[Variants.BH]: "bg-bh-titlebar font-bh-sr",
				[Variants.YS]: "bg-ys-titlebar font-ys",
				[Variants.SR]: "bg-sr-titlebar titlebar-sr-noise font-bh-sr",
				[Variants.NAP]:
					"bg-nap-titlebar font-nap border-t-2 border-r-2 border-l-2 border-nap-border rounded-tl-xl",
			},
		},
	},
);

const titlebarButtonStyles = cva("h-10 w-10 flex items-center justify-center", {
	variants: {
		intent: {
			[Variants.BH]: "",
			[Variants.YS]:
				"border-3 p-0.5 border-[#888d8e] bg-[#ece5d8] hover:border-transparent hover:drop-shadow-lg hover:drop-shadow[#fdfdfe] rounded-full active:bg-[#9a947f] active:border-transparent",
			[Variants.SR]: "",
			[Variants.NAP]: "",
		},
	},
});

export default function Titlebar() {
	const activeGame = useGame();
	const assetPath = `src/assets/icon/${getActiveGameCode()}`;
	let [mouseDown, setMouseDown] = useState<boolean>(false);
	return (
		<div
			style={{ zIndex: 1001, color: "white" }}
			data-tauri-drag-region
			class={titlebarStyles({ game: activeGame })}
		>
			<h3 class="text-xl text-center" data-tauri-drag-region>
				Yoohoo!
			</h3>
			<div
				class={titlebarButtonStyles({ intent: useGame() })}
				onClick={() => closeApp()}
				onMouseDown={() => setMouseDown(true)}
				onMouseUp={() => setMouseDown(false)}
			>
				<img
					style={{ display: mouseDown ? "none" : "" }}
					src={`${assetPath}/close.svg`}
					width={18}
					height={18}
				/>
				<img
					style={{ display: mouseDown ? "" : "none" }}
					src={`${assetPath}/close-click.svg`}
					width={18}
					height={18}
				/>
			</div>
		</div>
	);
}

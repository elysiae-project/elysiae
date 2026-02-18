import { cva } from "class-variance-authority";
import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import { getActiveGameCode } from "../util/AppFunctions";
import { useState } from "preact/hooks";

const menuCloseStyles = cva(
	"h-10 w-10 flex items-center justify-center transition-all duration-50",
	{
		variants: {
			game: {
				// Additional Styles aren't needed for bh or sr as they are just the icon
				[Variants.BH]: "",
				[Variants.YS]:
					"border-3 p-0.5 border-ys-btnborder bg-[#ece5d8] hover:border-transparent hover:drop-shadow-lg hover:drop-shadow[#fdfdfe] rounded-full active:bg-ys-btn-active-bg active:border-transparent",
				[Variants.SR]: "",
				[Variants.NAP]:
					"rounded-full nap-dots border-3 border-nap-btn-border-strong active:animate-nap-pulsate active:border-transparent",
			},
		},
	},
);

export default function MenuClose({
	clickAction,
}: {
	clickAction: () => void;
}) {
	const {game, setGame} = useGame();
	const assetPath = `src/assets/icon/${getActiveGameCode()}`;
	let [mouseDown, setMouseDown] = useState<boolean>(false);

	return (
		<div
			class={menuCloseStyles({ game: game })}
			onClick={() => clickAction()}
			onPointerDown={(e) => {
				setMouseDown(true);
				e.currentTarget.setPointerCapture(e.pointerId);
			}}
			onPointerUp={(e) => {
				setMouseDown(false);
				e.currentTarget.releasePointerCapture(e.pointerId);
			}}
			onPointerLeave={(e) => {
				setMouseDown(false);
				if (e.currentTarget.hasPointerCapture(e.pointerId)) {
					e.currentTarget.releasePointerCapture(e.pointerId);
				}
			}}
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
	);
}

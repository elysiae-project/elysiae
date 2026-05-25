import { cva } from "class-variance-authority";
import { useState } from "preact/hooks";
import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import { variantToGameCode } from "../util/AppFunctions";

const menuCloseStyles = cva(
	"flex items-center justify-center transition-all duration-75 h-10 w-10",
	{
		variants: {
			game: {
				// Additional Styles aren't needed for bh3 or hkrpg as they are just the icon
				[Variants.BH3]: null,
				[Variants.HK4E]:
					"border-3 p-0.5 border-hk4e-btnborder bg-hk4e-item-active-bg hover:border-transparent rounded-full active:bg-hk4e-btn-active-bg active:border-transparent",
				[Variants.HKRPG]: null,
				[Variants.NAP]:
					"rounded-full nap-dots-titlebar-btn border-3 border-nap-btn-border-strong active:animate-nap-pulsate active:border-transparent",
			},
		},
	},
);

export const MenuClose = ({
	clickAction,
	size = 32,
}: {
	clickAction: () => void;
	size?: number;
	width?: number;
	height?: number;
}) => {
	const { game } = useGame();
	const assetPath = `/icon/${variantToGameCode[game]}`;
	const [mouseDown, setMouseDown] = useState<boolean>(false);

	return (
		<button
			type="button"
			style={{ width: `${size}px`, height: `${size}px` }}
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
				width={size / 2.33}
				height={size / 2.33}
				alt="Close Window Button"
			/>
			<img
				style={{ display: mouseDown ? "" : "none" }}
				src={`${assetPath}/close-click.svg`}
				width={size / 2.33}
				height={size / 2.33}
				alt="Close Window Button"
			/>
		</button>
	);
};

export default MenuClose;

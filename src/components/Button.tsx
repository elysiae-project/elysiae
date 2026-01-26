import { useGame } from "../hooks/useGame";
import { Variants } from "../types";

import YsButton from "./ys/Button";
import BhButton from "./bh/Button";
import SrButton from "./sr/Button";
import NapButton from "./nap/Button";

export default function Button({
	onClick,
	children,
	intent,
}: {
	onClick: () => void;
	children: any;
	intent: "primary" | "secondary" | null | undefined;
}) {
	const game = useGame();
	const Button = {
		[Variants.BH]: BhButton,
		[Variants.YS]: YsButton,
		[Variants.SR]: SrButton,
		[Variants.NAP]: NapButton,
	}[game];
	return (
		<Button onClick={onClick} intent={intent}>
			{children}
		</Button>
	);
}

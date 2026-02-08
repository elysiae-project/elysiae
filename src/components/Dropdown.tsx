import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import BhDropdown from "./bh/Dropdown";
import NapDropdown from "./nap/Dropdown";
import SrDropdown from "./sr/Dropdown";
import YsDropdown from "./ys/Dropdown";

export default function Dropdown({
	labels,
	initialIndex = 0,
	onChangeAction,
}: {
	labels: string[];
	initialIndex: number;
	onChangeAction: (index: number) => void;
}) {
	const game = useGame();
	const Dropdown = {
		[Variants.BH]: BhDropdown,
		[Variants.YS]: YsDropdown,
		[Variants.SR]: SrDropdown,
		[Variants.NAP]: NapDropdown,
	}[game];
	return <Dropdown labels={labels} initialIndex={initialIndex} onChangeAction={(e) => {onChangeAction(e)}}></Dropdown>;
}

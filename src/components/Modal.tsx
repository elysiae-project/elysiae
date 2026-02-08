import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import BhModal from "./bh/Modal";
import NapModal from "./nap/Modal";
import SrModal from "./sr/Modal";
import YsModal from "./ys/Modal";

export default function Modal({
	children,
	title,
	open,
}: {
	children: any;
	title: string;
	open: boolean;
}) {
	const game = useGame();
	const Modal = {
		[Variants.BH]: BhModal,
		[Variants.YS]: YsModal,
		[Variants.SR]: SrModal,
		[Variants.NAP]: NapModal,
	}[game];
	return (
		<Modal title={title} open={open}>
			{children}
		</Modal>
	);
}

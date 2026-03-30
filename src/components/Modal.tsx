import { useEffect, useState } from "preact/hooks";
import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import { cva } from "class-variance-authority";
import MenuClose from "./MenuClose";

const modalStyles = cva(
	"px-5 py-3 overflow-y-scroll w-auto min-w-125 h-auto min-h-75",
	{
		variants: {
			game: {
				[Variants.BH3]: "bg-bh-modal-bg rounded-lg",
				[Variants.HK4E]: "bg-ys-modal-bg rounded-md text-white",
				[Variants.HKRPG]: "bg-sr-modal-bg rounded-md",
				[Variants.NAP]: "nap-dots rounded-xl border-4 border-nap-btn-border",
			},
		},
	},
);

const modalTitlebarStyles = cva(
	"flex flex-row justify-between items-center w-full mb-1 border-b",
	{
		variants: {
			game: {
				[Variants.BH3]: "border-b-white",
				[Variants.HK4E]:
					"text-ys-modal-titlebar-text border-b-ys-modal-titlebar-border",
				[Variants.HKRPG]: "border-b-black",
				[Variants.NAP]: "border-b-white",
			},
		},
	},
);

export default function Modal({
	children,
	title,
	open,
	onOpenUpdate = () => {
		// Extremely hacky way to fix an issue with the implementation of modals
		// Do not use this prop please
		open = !open;
	},
}: {
	children: any;
	title: string;
	open: boolean;
	onOpenUpdate?: () => void;
}) {
	const { game } = useGame();
	let [isOpen, setIsOpen] = useState<boolean>(open);

	const updateOpenState = () => {
		setIsOpen(false);
		onOpenUpdate();
	};

	useEffect(() => {
		updateOpenState();
	}, [open]);

	if (!isOpen) return;

	return (
		<div
			class="fixed inset-0 flex h-full w-full items-center justify-center"
			style={{
				backdropFilter: "blur(7px)",
				backgroundColor: "rgba(13,13,13,0.6)",
				zIndex: 1000,
				pointerEvents: "none",
			}}
			onClick={() => updateOpenState()}
		>
			<div class={modalStyles({ game: game })}>
				<div class={modalTitlebarStyles({ game: game })}>
					<h2>{title}</h2>
					<MenuClose clickAction={() => setIsOpen(false)} />
				</div>
				{children}
			</div>
		</div>
	);
}

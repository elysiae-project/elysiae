import { useEffect, useState } from "preact/hooks";
import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import { cva } from "class-variance-authority";
import { X } from "lucide-preact";

const modalStyles = cva(
	"px-5 py-3 overflow-y-scroll w-auto min-w-125 h-auto min-h-75",
	{
		variants: {
			game: {
				[Variants.BH]: "",
				[Variants.YS]: "bg-[#3b4354] rounded-md text-white",
				[Variants.SR]: "",
				[Variants.NAP]: "",
			},
		},
	},
);

const modalTitlebarStyles = cva(
	"flex flex-row justify-between items-center w-full mb-1",
	{
		variants: {
			game: {
				[Variants.BH]: "",
				[Variants.YS]: "text-[#d2bc8d] border-b border-b-[#69758f]",
				[Variants.SR]: "",
				[Variants.NAP]: "",
			},
		},
	},
);

function ModalCloseButton() {
	
}

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
	const activeGame = useGame();
	let [isOpen, setIsOpen] = useState(open);

	const updateOpenState = () => {
		setIsOpen(!isOpen);
		onOpenUpdate();
	};

	useEffect(() => {
		updateOpenState();
	}, [open]);

	if (!isOpen) return null;

	return (
		<div
			class="w-full h-full flex justify-center items-center fixed inset-0"
			style={{
				backdropFilter: "blur(7px)",
				backgroundColor: "rgba(13,13,13,0.6)",
				zIndex: 1000,
				pointerEvents: "none",
			}}
			onClick={() => updateOpenState()}
		>
			<div class={modalStyles({ game: activeGame })}>
				<div class={modalTitlebarStyles({ game: activeGame })}>
					<h2>{title}</h2>
					{/* TODO: Replace Lucide X component with the appropriate close button asset */}
					<X size={18} />
				</div>
				{children}
			</div>
		</div>
	);
}

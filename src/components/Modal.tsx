import { useGame } from "../hooks/useGame";
import { Variants } from "../types";
import { cva } from "class-variance-authority";
import MenuClose from "./MenuClose";

const modalStyles = cva(
	"px-5 py-3 overflow-y-scroll w-[55%] min-w-125 h-auto min-h-75 ",
	{
		variants: {
			game: {
				[Variants.BH3]: "bg-bh-modal-bg rounded-lg",
				[Variants.HK4E]: "bg-ys-modal-bg rounded-md text-white",
				[Variants.HKRPG]: "bg-sr-modal-bg rounded-md",
				[Variants.NAP]:
					"nap-dots rounded-br-2xl rounded-tl-2xl border-[0.195rem] border-nap-btn-border",
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
	onOpenUpdate,
}: {
	children: React.ReactNode;
	title: string;
	open: boolean;
	onOpenUpdate: () => void;
}) {
	const { game } = useGame();

	if (!open) return null;

	return (
		<div
			class="absolute inset-0 z-[1000] flex h-full w-full items-center justify-center"
			style={{
				backdropFilter: "blur(7px)",
				backgroundColor: "rgba(13,13,13,0.6)",
			}}
			onClick={onOpenUpdate}
		>
			<div class={modalStyles({ game })} onClick={(e) => e.stopPropagation()}>
				<div className={modalTitlebarStyles({ game })}>
					<h2>{title}</h2>
					<MenuClose clickAction={onOpenUpdate} />
				</div>
				<div class="w-full h-full">{children}</div>
			</div>
		</div>
	);
}

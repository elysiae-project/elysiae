import { useGame } from "../hooks/useGame";
import { ModalHandle, ModalProps, Variants } from "../types";
import { cva } from "class-variance-authority";
import MenuClose from "./MenuClose";
import { useImperativeHandle, useState } from "preact/hooks";
import { forwardRef } from "preact/compat";

const modalStyles = cva("overflow-y-scroll w-[65%] h-auto min-h-90", {
	variants: {
		game: {
			[Variants.BH3]: "bg-bh3-modal-bg rounded-lg",
			[Variants.HK4E]: "bg-hk4e-modal-bg rounded-md text-white",
			[Variants.HKRPG]: "bg-hkrpg-modal-bg rounded-md",
			[Variants.NAP]:
				"nap-dots rounded-br-2xl rounded-tl-2xl border-[0.195rem] border-nap-btn-border",
		},
	},
});

const modalTitlebarStyles = cva(
	"flex flex-row justify-between items-center mb-3 text-center border-b-2 w-full px-4 py-0.5 ",
	{
		variants: {
			game: {
				[Variants.BH3]: "border-b-white",
				[Variants.HK4E]:
					"text-hk4e-modal-titlebar-text border-b-hk4e-modal-titlebar-border",
				[Variants.HKRPG]: "border-b-black",
				[Variants.NAP]: "border-b-white",
			},
		},
	},
);

export const Modal = forwardRef<ModalHandle, ModalProps>(function Modal(
	{ title, children }: { title: string; children: React.ReactNode },
	ref,
) {
	const { game } = useGame();
	const [isOpen, setIsOpen] = useState<boolean>(false);

	useImperativeHandle(ref, () => ({
		open: () => setIsOpen(true),
		close: () => setIsOpen(false),
		toggle: () => setIsOpen((state: boolean) => !state),
	}));

	if (!isOpen) return null;
	return (
		<div
			class="absolute inset-0 z-60 flex h-full w-full items-center justify-center bg-black/50 backdrop-blur-xl"
			onClick={() => setIsOpen(false)}>
			<div class={modalStyles({ game })} onClick={(e) => e.stopPropagation()}>
				<div className={modalTitlebarStyles({ game })}>
					<h1 class="text-xl text-center">{title}</h1>
					<MenuClose clickAction={() => setIsOpen(false)} />
				</div>
				<div class="w-full h-full px-3 items-center">{children}</div>
			</div>
		</div>
	);
});

export default Modal;

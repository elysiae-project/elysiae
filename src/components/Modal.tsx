import { cva } from "class-variance-authority";
import { motion } from "motion/react";
import { forwardRef } from "preact/compat";
import { useImperativeHandle, useState } from "preact/hooks";
import { useGame } from "../hooks/useGame";
import { type ModalHandle, type ModalProps, Variants } from "../types";
import MenuClose from "./MenuClose";

const modalStyles = cva(
	"overflow-y-scroll w-[55%] min-w-125 h-auto min-h-75 px-5 py-3",
	{
		variants: {
			game: {
				[Variants.BH3]:
					"bg-bh3-modal-bg rounded-lg border border-bh3-modal-border",
				[Variants.HK4E]: "bg-hk4e-modal-bg rounded-md text-white",
				[Variants.HKRPG]: "bg-hkrpg-modal-bg rounded-md",
				[Variants.NAP]:
					"nap-dots-titlebar bg-nap-titlebar rounded-br-2xl rounded-tl-2xl border-[0.195rem] border-nap-btn-border",
			},
		},
	},
);

const modalTitlebarStyles = cva(
	"flex flex-row justify-between items-center w-full mb-1 pb-1.5",
	{
		variants: {
			game: {
				[Variants.BH3]: "border-b-bh3-modal-border",
				[Variants.HK4E]:
					"text-hk4e-modal-titlebar-text border-b-hk4e-modal-titlebar-border",
				[Variants.HKRPG]: "border-b-black",
				[Variants.NAP]: "border-b-white",
			},
		},
	},
);

export const Modal = forwardRef<ModalHandle, ModalProps>(
	(
		{
			title = "",
			closeable = true,
			children,
			width = 750,
			height = 250,
		}: ModalProps,
		ref,
	) => {
		const { game } = useGame();
		const [isOpen, setIsOpen] = useState<boolean>(false);

		useImperativeHandle(ref, () => ({
			open: () => setIsOpen(true),
			close: () => setIsOpen(false),
			toggle: (state: boolean) => setIsOpen(state),
		}));

		if (!isOpen) return null;

		return (
			<button
				type="button"
				class="absolute inset-0 z-20 flex h-full w-full items-center justify-center bg-black/50 backdrop-blur-md"
				onClick={() => {
					if (closeable) setIsOpen(false);
				}}
			>
				<motion.button
					initial={{ y: 100, opacity: 0 }}
					animate={{
						y: 0,
						opacity: 1,
						transition: { duration: 0.15 },
					}}
					exit={{ scale: 0, transition: { duration: 0.1 } }}
					type="button"
					class={modalStyles({ game })}
					style={{ minWidth: `${width}px`, minHeight: `${height}px` }}
					onClick={(e) => e.stopPropagation()}
				>
					<div
						class={`${modalTitlebarStyles({ game })} ${closeable ? "border-b" : ""}`}
					>
						<h2>{title}</h2>
						{closeable ? (
							<MenuClose clickAction={() => setIsOpen(false)} />
						) : null}
					</div>
					<div class="h-full w-full">{children}</div>
				</motion.button>
			</button>
		);
	},
);

export default Modal;

import { cva } from "class-variance-authority";

const button = cva("rounded-full font-ys min-w-55 transition-colors duration-150 px-5 py-2", {
    variants: {
        intent: {
            primary: "bg-[#e3ddcf] text-[#4a5264] border-2 border-[#00000000] hover:border-white hover:inset-shadow-[#d9d3c9] active:border-[#7e7e82] active:bg-[#cbbb9d] active:text-white",
            secondary: "bg-[#4a5264] text-[#e3ddcf] border-2 border-[#00000000] hover:border-[#f9e5b0] active:bg-[#fbebca] active:text-[#9c8e7a] active:border-[#b7b2ab]"
        }
    }
})

export default function YsButton({
	onClick,
	children,
	intent,
}: {
	onClick: () => void;
	children: any;
	intent: 'primary' | 'secondary' | null | undefined;
}) {
    return(
        <div class={button({intent: intent})} onClick={onClick}>
            {children}
        </div>
    )
}

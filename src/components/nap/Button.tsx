import { cva } from "class-variance-authority";

const button = cva("", {
    variants: {
        intent: {
            primary: "",
            secondary: ""
        }
    }
})

export default function NapButton({
    onClick,
    children,
    intent,
}: {
    onClick: () => void;
    children: any;
    intent: "primary" | "secondary" | null | undefined;
}) {
    return(
        <div class={button({ intent: intent })} onClick={onClick}>
            {children}
        </div>
    )
}

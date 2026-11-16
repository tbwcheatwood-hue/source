import { useMemo, useState } from "react";

type Op = "+" | "-" | "×" | "÷" | null;

function apply(a: number, b: number, op: Op): number {
  switch (op) {
    case "+":
      return a + b;
    case "-":
      return a - b;
    case "×":
      return a * b;
    case "÷":
      return b === 0 ? NaN : a / b;
    default:
      return b;
  }
}

function format(n: number): string {
  if (Number.isNaN(n)) return "Error";
  if (!Number.isFinite(n)) return "∞";
  const s = n.toPrecision(12).replace(/\.?0+$/, "");
  return s.length > 14 ? n.toExponential(6) : s;
}

/** Calculator-class offline UI — complete, not a stub. Customize for the user's app. */
export function App() {
  const [display, setDisplay] = useState("0");
  const [acc, setAcc] = useState<number | null>(null);
  const [op, setOp] = useState<Op>(null);
  const [fresh, setFresh] = useState(true);

  const subtitle = useMemo(
    () => (op && acc !== null ? `${format(acc)} ${op}` : "Offline · Tauri v2"),
    [acc, op],
  );

  const inputDigit = (d: string) => {
    setDisplay((cur) => {
      if (fresh || cur === "0" || cur === "Error") {
        setFresh(false);
        return d;
      }
      if (cur.length >= 14) return cur;
      return cur + d;
    });
  };

  const inputDot = () => {
    setDisplay((cur) => {
      if (fresh || cur === "Error") {
        setFresh(false);
        return "0.";
      }
      return cur.includes(".") ? cur : cur + ".";
    });
  };

  const clearAll = () => {
    setDisplay("0");
    setAcc(null);
    setOp(null);
    setFresh(true);
  };

  const chooseOp = (next: Op) => {
    const cur = Number(display);
    if (acc !== null && op && !fresh) {
      const result = apply(acc, cur, op);
      setAcc(result);
      setDisplay(format(result));
    } else {
      setAcc(cur);
    }
    setOp(next);
    setFresh(true);
  };

  const equals = () => {
    if (acc === null || !op) return;
    const result = apply(acc, Number(display), op);
    setDisplay(format(result));
    setAcc(null);
    setOp(null);
    setFresh(true);
  };

  const keys: { label: string; onClick: () => void; className?: string }[] = [
    { label: "C", onClick: clearAll, className: "muted" },
    {
      label: "±",
      onClick: () =>
        setDisplay((cur) =>
          cur === "0" || cur === "Error" ? cur : cur.startsWith("-") ? cur.slice(1) : `-${cur}`,
        ),
      className: "muted",
    },
    {
      label: "%",
      onClick: () => setDisplay((cur) => format(Number(cur) / 100)),
      className: "muted",
    },
    { label: "÷", onClick: () => chooseOp("÷"), className: "op" },
    { label: "7", onClick: () => inputDigit("7") },
    { label: "8", onClick: () => inputDigit("8") },
    { label: "9", onClick: () => inputDigit("9") },
    { label: "×", onClick: () => chooseOp("×"), className: "op" },
    { label: "4", onClick: () => inputDigit("4") },
    { label: "5", onClick: () => inputDigit("5") },
    { label: "6", onClick: () => inputDigit("6") },
    { label: "-", onClick: () => chooseOp("-"), className: "op" },
    { label: "1", onClick: () => inputDigit("1") },
    { label: "2", onClick: () => inputDigit("2") },
    { label: "3", onClick: () => inputDigit("3") },
    { label: "+", onClick: () => chooseOp("+"), className: "op" },
    { label: "0", onClick: () => inputDigit("0"), className: "zero" },
    { label: ".", onClick: inputDot },
    { label: "=", onClick: equals, className: "op" },
  ];

  return (
    <main className="shell">
      <header className="brand">
        <p className="product">Source Desktop</p>
        <p className="hint">{subtitle}</p>
      </header>
      <section className="calc" aria-label="Calculator">
        <output className="display" aria-live="polite">
          {display}
        </output>
        <div className="pad">
          {keys.map((k) => (
            <button
              key={k.label}
              type="button"
              className={k.className}
              onClick={k.onClick}
            >
              {k.label}
            </button>
          ))}
        </div>
      </section>
    </main>
  );
}

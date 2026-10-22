import type { IndicatorInput, IndicatorInputs, IndicatorInputValue } from "../indicators/contracts.js";
import { configInput } from "./config-dialog.js";
export function IndicatorInputFields({ schema, value, onChange, disabled = false }: { schema: IndicatorInput[]; value: IndicatorInputs; onChange: (value: IndicatorInputs) => void; disabled?: boolean }) {
  return <fieldset disabled={disabled} className="space-y-3">{schema.map((input) => {
    const actual = Object.hasOwn(value, input.key) ? value[input.key]! : input.default;
    const change = (item: IndicatorInputValue) => onChange({ ...value, [input.key]: item });
    return <div key={input.key} className="space-y-1 text-sm"><label className="block">{input.title} <span className="text-xs text-zinc-500">({input.key}; default {String(input.default)})</span>
      {input.options ? <select className={`${configInput} cursor-pointer`} value={String(actual)} onChange={(e) => change(input.options!.find((v) => String(v) === e.target.value)!)}>{input.options.map((option) => <option key={String(option)} value={String(option)}>{String(option)}</option>)}</select>
        : input.type === "bool" ? <input className="ml-2 cursor-pointer" type="checkbox" checked={actual === true} onChange={(e) => change(e.target.checked)} />
        : <input className={configInput} type={input.type === "int" || input.type === "float" ? "number" : "text"} value={String(actual)} min={input.min} max={input.max} step={input.step ?? (input.type === "int" ? 1 : "any")} onChange={(e) => { const next = input.type === "int" || input.type === "float" ? Number(e.target.value) : e.target.value; change(next); }} />}</label>
      {input.tooltip && <p className="text-xs text-zinc-400">{input.tooltip}</p>}
      {Object.hasOwn(value, input.key) && <button type="button" className="cursor-pointer text-xs underline" onClick={() => onChange(Object.fromEntries(Object.entries(value).filter(([key]) => key !== input.key)))}>Use source default</button>}
    </div>;
  })}</fieldset>;
}

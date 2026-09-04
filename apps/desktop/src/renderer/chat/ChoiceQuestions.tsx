import { Button, Checkbox, Input, Radio, Tag, Typography } from "antd";
import { useState } from "react";
import type { ReactNode } from "react";
import type { ChoiceGroup } from "../types";

type ChoiceAnswer = {
  selectedOptions: string[];
  details: Record<string, string>;
  customSelected: boolean;
  customOption: string;
  customDetail: string;
};

export default function ChoiceQuestions({
  groups,
  disabled,
  onSubmit,
}: {
  groups: ChoiceGroup[];
  disabled: boolean;
  onSubmit: (content: string) => Promise<void>;
}) {
  const [answers, setAnswers] = useState<ChoiceAnswer[]>(() =>
    groups.map(() => ({
      selectedOptions: [],
      details: {},
      customSelected: false,
      customOption: "",
      customDetail: "",
    })),
  );

  const updateAnswer = (
    groupIndex: number,
    update: (answer: ChoiceAnswer) => ChoiceAnswer,
  ) => {
    setAnswers((current) =>
      current.map((answer, index) =>
        index === groupIndex ? update(answer) : answer,
      ),
    );
  };

  const selectOption = (
    groupIndex: number,
    group: ChoiceGroup,
    option: string,
    selected: boolean,
  ) => {
    updateAnswer(groupIndex, (answer) => ({
      ...answer,
      selectedOptions: group.multiple
        ? selected
          ? [...new Set([...answer.selectedOptions, option])]
          : answer.selectedOptions.filter((value) => value !== option)
        : selected
          ? [option]
          : [],
      customSelected: group.multiple ? answer.customSelected : false,
    }));
  };

  const selectCustom = (
    groupIndex: number,
    group: ChoiceGroup,
    selected: boolean,
  ) => {
    updateAnswer(groupIndex, (answer) => ({
      ...answer,
      selectedOptions: group.multiple ? answer.selectedOptions : [],
      customSelected: selected,
    }));
  };

  const complete = groups.every((_, index) => {
    const answer = answers[index];
    const hasSelection =
      answer.selectedOptions.length > 0 || answer.customSelected;
    return (
      hasSelection &&
      (!answer.customSelected || Boolean(answer.customOption.trim()))
    );
  });

  return (
    <div className="choice-questions">
      {groups.map((group, groupIndex) => {
        const answer = answers[groupIndex];
        const singleValue = answer.customSelected
          ? group.options.length
          : group.options.indexOf(answer.selectedOptions[0]);
        return (
          <fieldset
            className="choice-group"
            key={`${groupIndex}-${group.item}`}
          >
            <legend>
              <Typography.Text strong>{group.item}</Typography.Text>
              <Tag>{group.multiple ? "多选" : "单选"}</Tag>
            </legend>
            <div className="choice-options">
              {group.multiple ? (
                group.options.map((option) => (
                  <ChoiceOptionRow
                    key={option}
                    control={
                      <Checkbox
                        checked={answer.selectedOptions.includes(option)}
                        disabled={disabled}
                        onChange={(event) =>
                          selectOption(
                            groupIndex,
                            group,
                            option,
                            event.target.checked,
                          )
                        }
                      >
                        {option}
                        {group.recommended.includes(option) ? "（推荐）" : ""}
                      </Checkbox>
                    }
                    detail={answer.details[option] ?? ""}
                    disabled={disabled}
                    onDetailChange={(detail) =>
                      updateAnswer(groupIndex, (current) => ({
                        ...current,
                        selectedOptions: [
                          ...new Set([...current.selectedOptions, option]),
                        ],
                        details: { ...current.details, [option]: detail },
                      }))
                    }
                  />
                ))
              ) : (
                <Radio.Group
                  className="choice-radio-group"
                  value={singleValue}
                  onChange={(event) => {
                    const optionIndex = event.target.value as number;
                    if (optionIndex === group.options.length) {
                      selectCustom(groupIndex, group, true);
                      return;
                    }
                    selectOption(
                      groupIndex,
                      group,
                      group.options[optionIndex],
                      true,
                    );
                  }}
                >
                  {group.options.map((option, optionIndex) => (
                    <ChoiceOptionRow
                      key={option}
                      control={
                        <Radio value={optionIndex} disabled={disabled}>
                          {option}
                          {group.recommended.includes(option) ? "（推荐）" : ""}
                        </Radio>
                      }
                      detail={answer.details[option] ?? ""}
                      disabled={disabled}
                      onDetailChange={(detail) =>
                        updateAnswer(groupIndex, (current) => ({
                          ...current,
                          selectedOptions: [option],
                          customSelected: false,
                          details: { ...current.details, [option]: detail },
                        }))
                      }
                    />
                  ))}
                  <CustomChoiceRow
                    control={
                      <Radio value={group.options.length} disabled={disabled}>
                        其他（自定义）
                      </Radio>
                    }
                    answer={answer}
                    disabled={disabled}
                    onSelect={() => selectCustom(groupIndex, group, true)}
                    onChange={(change) =>
                      updateAnswer(groupIndex, (current) => ({
                        ...current,
                        selectedOptions: [],
                        customSelected: true,
                        ...change,
                      }))
                    }
                  />
                </Radio.Group>
              )}
              {group.multiple && (
                <CustomChoiceRow
                  control={
                    <Checkbox
                      checked={answer.customSelected}
                      disabled={disabled}
                      onChange={(event) =>
                        selectCustom(groupIndex, group, event.target.checked)
                      }
                    >
                      其他（自定义）
                    </Checkbox>
                  }
                  answer={answer}
                  disabled={disabled}
                  onSelect={() => selectCustom(groupIndex, group, true)}
                  onChange={(change) =>
                    updateAnswer(groupIndex, (current) => ({
                      ...current,
                      customSelected: true,
                      ...change,
                    }))
                  }
                />
              )}
            </div>
          </fieldset>
        );
      })}
      <Button
        type="primary"
        disabled={disabled || !complete}
        onClick={() => void onSubmit(formatChoiceAnswers(groups, answers))}
      >
        提交选择
      </Button>
    </div>
  );
}

function ChoiceOptionRow({
  control,
  detail,
  disabled,
  onDetailChange,
}: {
  control: ReactNode;
  detail: string;
  disabled: boolean;
  onDetailChange: (detail: string) => void;
}) {
  return (
    <div className="choice-option-row">
      <div className="choice-option-control">{control}</div>
      <Input
        value={detail}
        disabled={disabled}
        placeholder="补充说明（可选）"
        onChange={(event) => onDetailChange(event.target.value)}
      />
    </div>
  );
}

function CustomChoiceRow({
  control,
  answer,
  disabled,
  onSelect,
  onChange,
}: {
  control: ReactNode;
  answer: ChoiceAnswer;
  disabled: boolean;
  onSelect: () => void;
  onChange: (change: Partial<ChoiceAnswer>) => void;
}) {
  return (
    <div className="choice-option-row choice-custom-option">
      <div className="choice-option-control">{control}</div>
      <Input
        value={answer.customOption}
        disabled={disabled}
        placeholder="输入自定义选项"
        onFocus={onSelect}
        onChange={(event) => onChange({ customOption: event.target.value })}
      />
      <Input
        value={answer.customDetail}
        disabled={disabled}
        placeholder="补充说明（可选）"
        onFocus={onSelect}
        onChange={(event) => onChange({ customDetail: event.target.value })}
      />
    </div>
  );
}

export function formatChoiceAnswers(
  groups: ChoiceGroup[],
  answers: ChoiceAnswer[],
) {
  const lines = ["我的选择："];
  groups.forEach((group, index) => {
    const answer = answers[index];
    lines.push(`${index + 1}. ${group.item}`);
    for (const option of answer.selectedOptions) {
      const detail = answer.details[option]?.trim();
      lines.push(`- ${option}${detail ? `；补充：${detail}` : ""}`);
    }
    if (answer.customSelected) {
      const detail = answer.customDetail.trim();
      lines.push(
        `- ${answer.customOption.trim()}${detail ? `；补充：${detail}` : ""}`,
      );
    }
  });
  return lines.join("\n");
}

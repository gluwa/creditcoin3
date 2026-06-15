export interface ContentCheck {
    type: 'graphql-entity-modified' | 'graphql-entity-added-removed';
    file: string;
}

export interface WarningRule {
    id: string;
    label: string;
    description: string;
    severity: 'warning' | 'critical';
    include: string[];
    exclude?: string[];
    contentCheck?: ContentCheck;
}

export interface WarningRulesConfig {
    rules: WarningRule[];
}

export interface TriggeredWarning {
    rule: WarningRule;
    matchedFiles: string[];
}

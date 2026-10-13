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

export interface ReleaseTopic {
    id: string;
    label: string;
    scopes?: string[];
    types?: string[];
    include?: string[];
}

export interface DependencyTopic {
    label: string;
    types?: string[];
    scopes?: string[];
    authors?: string[];
}

export interface ReleaseTopicsConfig {
    topics: ReleaseTopic[];
    dependencies: DependencyTopic;
    skip?: string[];
}

export interface ReleaseChange {
    sha: string;
    title: string;
    author: string;
    login?: string;
    pr?: number;
    type?: string;
    scopes: string[];
    files: string[];
}

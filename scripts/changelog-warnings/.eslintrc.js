module.exports = {
    env: {
        node: true,
        es2022: true,
    },
    extends: ['plugin:@typescript-eslint/recommended', 'prettier'],
    parser: '@typescript-eslint/parser',
    parserOptions: {
        project: 'tsconfig.json',
        sourceType: 'module',
    },
    plugins: ['@typescript-eslint'],
    rules: {
        'no-console': 'off',
        '@typescript-eslint/no-unused-vars': [
            'warn',
            {
                argsIgnorePattern: '^_',
                varsIgnorePattern: '^_',
            },
        ],
        'no-var': 'error',
        'prefer-const': 'error',
        eqeqeq: ['error', 'smart'],
        'no-throw-literal': 'error',
        'prefer-arrow-callback': 'error',
    },
    root: true,
};

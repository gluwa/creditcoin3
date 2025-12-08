module.exports = {
    env: {
        node: true,
        es2022: true,
    },
    extends: ['eslint:recommended', 'prettier'],
    parserOptions: {
        ecmaVersion: 2022,
        sourceType: 'module',
    },
    rules: {
        'no-console': 'off', // Allow console.log in scripts
        'no-unused-vars': [
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
};

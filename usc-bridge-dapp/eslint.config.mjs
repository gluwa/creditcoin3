// eslint-config-next ships native flat configs now — no @eslint/eslintrc FlatCompat bridging
// needed (that legacy `.extends()` path crashes here with a circular-structure error, since
// eslint-plugin-react's flat `configs.flat` self-references in a way the old compat validator
// chokes on).
import nextCoreWebVitals from "eslint-config-next/core-web-vitals";
import nextTypescript from "eslint-config-next/typescript";

const eslintConfig = [...nextCoreWebVitals, ...nextTypescript];

export default eslintConfig;

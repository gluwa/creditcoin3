# Description of proposed changes

<describe what this PR is about and why we want it>

---

When merging this PR:

- For PRs against `usc-dev` use the "Squash and merge" button
- For PRs against `usc-testnet`/`main` use the "Create a merge commit" button
- Hotfixes against `usc-testnet`/`main` should use the "Squash and merge" button

---

Practical tips for PR review:

- [ ] All GitHub Actions report PASS
- [ ] Newly added code/functions have unit tests
  - [ ] Coverage tools report all newly added lines as covered
  - [ ] The positive scenario is exercised
  - [ ] Negative scenarios are exercised, e.g. assert on all possible errors
  - [ ] Assert on events triggered if applicable
  - [ ] Assert on changes made to storage if applicable
- [ ] Modified behavior/functions - try to make sure above test items are covered
- [ ] Integration tests are added if applicable/needed

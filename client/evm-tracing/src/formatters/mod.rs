// Copyright 2019-2022 PureStake Inc.
// This file is part of Moonbeam.

// Moonbeam is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Moonbeam is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Moonbeam.  If not, see <http://www.gnu.org/licenses/>.

pub mod blockscout;
pub mod call_tracer;
pub mod raw;
pub mod trace_filter;

pub use blockscout::Formatter as Blockscout;
pub use call_tracer::Formatter as CallTracer;
pub use raw::Formatter as Raw;
pub use trace_filter::Formatter as TraceFilter;

use evm_tracing_events::Listener;
use serde::Serialize;

pub trait ResponseFormatter {
	type Listener: Listener;
	type Response: Serialize;

	fn format(listener: Self::Listener) -> Option<Self::Response>;
}

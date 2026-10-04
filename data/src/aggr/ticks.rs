//! 【zh】 按成交笔数（tick）聚合 K 线与足迹图（footprint）的模块。
//! 【zh】 与按时间聚合不同，每根 K 线固定包含 N 笔成交，N 由 `TickCount` 决定。
use crate::aggr;
use crate::chart::kline::{ClusterKind, KlineTrades, NPoc};
use exchange::unit::Qty;
use exchange::unit::price::{Price, PriceStep};
use exchange::{Kline, Trade, Volume};

use std::collections::BTreeMap;

// 【zh】 单根 tick K 线的累积状态：已收成交笔数、OHLC 与成交量，
// 【zh】 以及按价格档位分组的足迹数据（`KlineTrades`）。
#[derive(Debug, Clone)]
pub struct TickAccumulation {
    pub tick_count: usize,
    pub kline: Kline,
    pub footprint: KlineTrades,
}

impl TickAccumulation {
    // 【zh】 用第一笔成交创建新的累积：开高低收都等于该成交价，
    // 【zh】 成交量按买卖方向初始化，并把成交归入最近的价格档位。
    pub fn new(trade: &Trade, step: PriceStep) -> Self {
        let mut footprint = KlineTrades::new();
        footprint.add_trade_to_nearest_bin(trade, step);

        let kline = Kline {
            time: trade.time,
            open: trade.price,
            high: trade.price,
            low: trade.price,
            close: trade.price,
            volume: Volume::empty_buy_sell().add_trade_qty(trade.is_sell, trade.qty),
        };

        Self {
            tick_count: 1,
            kline,
            footprint,
        }
    }

    // 【zh】 并入后续成交：计数加一，更新最高/最低/收盘价与买卖量，
    // 【zh】 同时写入足迹。注意不在此重算 POC，由调用方批量处理。
    pub fn update_with_trade(&mut self, trade: &Trade, step: PriceStep) {
        self.tick_count += 1;
        self.kline.high = self.kline.high.max(trade.price);
        self.kline.low = self.kline.low.min(trade.price);
        self.kline.close = trade.price;

        self.kline.volume = self.kline.volume.add_trade_qty(trade.is_sell, trade.qty);

        self.add_trade(trade, step);
    }

    fn add_trade(&mut self, trade: &Trade, step: PriceStep) {
        self.footprint.add_trade_to_nearest_bin(trade, step);
    }

    pub fn max_cluster_qty(&self, cluster_kind: ClusterKind, highest: Price, lowest: Price) -> Qty {
        self.footprint
            .max_cluster_qty(cluster_kind, highest, lowest)
    }

    pub fn is_full(&self, interval: aggr::TickCount) -> bool {
        self.tick_count >= interval.0 as usize
    }

    pub fn poc_price(&self) -> Option<Price> {
        self.footprint.poc_price()
    }

    pub fn set_poc_status(&mut self, status: NPoc) {
        self.footprint.set_poc_status(status);
    }

    pub fn calculate_poc(&mut self) {
        self.footprint.calculate_poc();
    }

    // 【zh】 返回买卖量差（delta）。优先用 K 线成交量；
    // 【zh】 若其不带方向，则退回到足迹各档位 delta 之和；都没有则为 0。
    pub fn volume_delta(&self) -> Qty {
        if self.kline.volume.is_directional() {
            self.kline.volume.delta()
        } else if !self.footprint.trades.is_empty() {
            self.footprint
                .trades
                .values()
                .fold(Qty::ZERO, |acc, group| acc + group.delta_qty())
        } else {
            Qty::ZERO
        }
    }

    /// Whether this tick accumulation has directional (buy vs sell) data.
    /// 【zh】 是否具备买卖方向数据：足迹非空，或 K 线成交量本身带方向。
    pub fn is_directional(&self) -> bool {
        !self.footprint.trades.is_empty() || self.kline.volume.is_directional()
    }
}

// 【zh】 tick 聚合器：按时间顺序保存所有 `TickAccumulation`，
// 【zh】 `interval` 为每根 K 线的成交笔数，`tick_size` 为足迹价格档位步长。
pub struct TickAggr {
    pub datapoints: Vec<TickAccumulation>,
    pub interval: aggr::TickCount,
    pub tick_size: PriceStep,
}

impl TickAggr {
    // 【zh】 创建聚合器；若有原始成交则立即按顺序聚合。
    pub fn new(interval: aggr::TickCount, tick_size: PriceStep, raw_trades: &[Trade]) -> Self {
        let mut tick_aggr = Self {
            datapoints: Vec::new(),
            interval,
            tick_size,
        };

        if !raw_trades.is_empty() {
            tick_aggr.insert_trades(raw_trades);
        }

        tick_aggr
    }

    // 【zh】 更改价格步长。分档结果依赖步长，无法增量调整，
    // 【zh】 所以清空已有数据并用原始成交整体重建。
    pub fn change_tick_size(&mut self, tick_size: PriceStep, raw_trades: &[Trade]) {
        self.tick_size = tick_size;

        self.datapoints.clear();

        if !raw_trades.is_empty() {
            self.insert_trades(raw_trades);
        }
    }

    /// return latest data point and its index
    /// 【zh】 返回最新的数据点及其下标；没有数据时为 `None`。
    pub fn latest_dp(&self) -> Option<(&TickAccumulation, usize)> {
        self.datapoints
            .last()
            .map(|dp| (dp, self.datapoints.len() - 1))
    }

    pub fn volume_data(&self) -> BTreeMap<u64, exchange::Volume> {
        self.into()
    }

    // 【zh】 按序写入成交：最后一根已满（达到 `interval`）就新开一根，否则并入。
    // 【zh】 记录被改动的下标，循环结束后统一重算 POC，避免每笔都重算；
    // 【zh】 最后刷新所有 POC 的未平仓状态。
    pub fn insert_trades(&mut self, buffer: &[Trade]) {
        let mut updated_indices = Vec::new();

        for trade in buffer {
            if self.datapoints.is_empty() {
                self.datapoints
                    .push(TickAccumulation::new(trade, self.tick_size));
                updated_indices.push(0);
            } else {
                let last_idx = self.datapoints.len() - 1;

                if self.datapoints[last_idx].is_full(self.interval) {
                    self.datapoints
                        .push(TickAccumulation::new(trade, self.tick_size));
                    updated_indices.push(self.datapoints.len() - 1);
                } else {
                    self.datapoints[last_idx].update_with_trade(trade, self.tick_size);
                    if !updated_indices.contains(&last_idx) {
                        updated_indices.push(last_idx);
                    }
                }
            }
        }

        for idx in updated_indices {
            if idx < self.datapoints.len() {
                self.datapoints[idx].calculate_poc();
            }
        }

        self.update_poc_status();
    }

    // 【zh】 计算每根 K 线 POC（控制点）是否被后续 K 线触及（naked POC）。
    // 【zh】 找到首根覆盖该价格的后续 K 线则标记为已填补，
    // 【zh】 其下标按渲染顺序反转（最新为 0）；否则为未填补。
    pub fn update_poc_status(&mut self) {
        let updates = self
            .datapoints
            .iter()
            .enumerate()
            .filter_map(|(idx, dp)| dp.poc_price().map(|price| (idx, price)))
            .collect::<Vec<_>>();

        let total_points = self.datapoints.len();

        for (current_idx, poc_price) in updates {
            let mut npoc = NPoc::default();

            for next_idx in (current_idx + 1)..total_points {
                let next_dp = &self.datapoints[next_idx];

                let next_dp_low = next_dp.kline.low.round_to_side_step(true, self.tick_size);
                let next_dp_high = next_dp.kline.high.round_to_side_step(false, self.tick_size);

                if next_dp_low <= poc_price && next_dp_high >= poc_price {
                    // on render we reverse the order of the points
                    // as it is easier to just take the idx=0 as latest candle for coords
                    let reversed_idx = (total_points - 1) - next_idx;
                    npoc.filled(reversed_idx as u64);
                    break;
                } else {
                    npoc.unfilled();
                }
            }

            if current_idx < total_points {
                let data_point = &mut self.datapoints[current_idx];
                data_point.set_poc_status(npoc);
            }
        }
    }

    // 【zh】 求区间内 K 线的最低价与最高价。下标以最新一根为 0 反向计数，
    // 【zh】 `earliest`/`latest` 均按此约定；区间非法或无数据返回 `None`。
    pub fn min_max_price_in_range_prices(
        &self,
        earliest: usize,
        latest: usize,
    ) -> Option<(Price, Price)> {
        if earliest > latest {
            return None;
        }

        let mut min_p: Option<Price> = None;
        let mut max_p: Option<Price> = None;

        self.datapoints
            .iter()
            .rev()
            .enumerate()
            .filter(|(idx, _)| *idx >= earliest && *idx <= latest)
            .for_each(|(_, dp)| {
                let low = dp.kline.low;
                let high = dp.kline.high;

                min_p = Some(match min_p {
                    Some(value) => value.min(low),
                    None => low,
                });
                max_p = Some(match max_p {
                    Some(value) => value.max(high),
                    None => high,
                });
            });

        match (min_p, max_p) {
            (Some(low), Some(high)) => Some((low, high)),
            _ => None,
        }
    }

    pub fn min_max_price_in_range(&self, earliest: usize, latest: usize) -> Option<(f32, f32)> {
        self.min_max_price_in_range_prices(earliest, latest)
            .map(|(min_p, max_p)| (min_p.to_f32_lossy(), max_p.to_f32_lossy()))
    }

    // 【zh】 与上一函数类似，但额外把足迹中的所有档位价格纳入比较，
    // 【zh】 用于足迹图纵轴范围。下标同样以最新一根为 0。
    pub fn min_max_footprint_price_in_range(
        &self,
        earliest: usize,
        latest: usize,
    ) -> Option<(Price, Price)> {
        if earliest > latest {
            return None;
        }

        let mut min_p: Option<Price> = None;
        let mut max_p: Option<Price> = None;

        self.datapoints
            .iter()
            .rev()
            .enumerate()
            .filter(|(idx, _)| *idx >= earliest && *idx <= latest)
            .for_each(|(_, dp)| {
                min_p = Some(match min_p {
                    Some(value) => value.min(dp.kline.low),
                    None => dp.kline.low,
                });
                max_p = Some(match max_p {
                    Some(value) => value.max(dp.kline.high),
                    None => dp.kline.high,
                });

                for price in dp.footprint.trades.keys() {
                    min_p = Some(match min_p {
                        Some(value) => value.min(*price),
                        None => *price,
                    });
                    max_p = Some(match max_p {
                        Some(value) => value.max(*price),
                        None => *price,
                    });
                }
            });

        match (min_p, max_p) {
            (Some(low), Some(high)) => Some((low, high)),
            _ => None,
        }
    }

    // 【zh】 求区间内（最新为 0 的下标）各 K 线在给定价格范围内的最大簇成交量，
    // 【zh】 用于足迹图中柱宽/颜色的归一化。
    pub fn max_qty_idx_range(
        &self,
        cluster_kind: ClusterKind,
        earliest: usize,
        latest: usize,
        highest: Price,
        lowest: Price,
    ) -> Qty {
        let mut max_cluster_qty: Qty = Qty::default();

        self.datapoints
            .iter()
            .rev()
            .enumerate()
            .filter(|(index, _)| *index <= latest && *index >= earliest)
            .for_each(|(_, dp)| {
                max_cluster_qty =
                    max_cluster_qty.max(dp.max_cluster_qty(cluster_kind, highest, lowest));
            });

        max_cluster_qty
    }
}

impl From<&TickAggr> for BTreeMap<u64, exchange::Volume> {
    /// Converts datapoints into a map of timestamps and volume data
    /// 【zh】 转为“下标 → 成交量”的映射。tick 图没有时间轴，故以数据点序号作键。
    fn from(tick_aggr: &TickAggr) -> Self {
        tick_aggr
            .datapoints
            .iter()
            .enumerate()
            .map(|(idx, dp)| (idx as u64, dp.kline.volume))
            .collect()
    }
}

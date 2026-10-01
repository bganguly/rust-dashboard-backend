use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionDTO {
    pub id: i32,
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerSummaryDTO {
    pub id: i32,
    pub email: String,
    #[serde(rename = "firstName")]
    pub first_name: String,
    #[serde(rename = "lastName")]
    pub last_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerDTO {
    pub id: i32,
    pub email: String,
    #[serde(rename = "firstName")]
    pub first_name: String,
    #[serde(rename = "lastName")]
    pub last_name: String,
    pub phone: Option<String>,
    pub region: RegionDTO,
    #[serde(rename = "createdAt")]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerListResult {
    pub data: Vec<CustomerDTO>,
    #[serde(rename = "nextCursor")]
    pub next_cursor: Option<i32>,
    #[serde(rename = "hasMore")]
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductSummaryDTO {
    pub id: i32,
    pub sku: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderItemDTO {
    pub id: i32,
    #[serde(rename = "productId")]
    pub product_id: i32,
    pub quantity: i32,
    #[serde(rename = "unitPrice")]
    pub unit_price: f64,
    pub discount: f64,
    pub product: ProductSummaryDTO,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderDTO {
    pub id: i32,
    pub status: String,
    pub total: f64,
    pub currency: String,
    pub notes: Option<String>,
    #[serde(rename = "placedAt")]
    pub placed_at: String,
    pub customer: CustomerSummaryDTO,
    pub region: RegionDTO,
    pub items: Vec<OrderItemDTO>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderListResult {
    pub data: Vec<OrderDTO>,
    pub page: i32,
    #[serde(rename = "pageSize")]
    pub page_size: i32,
    pub total: i64,
    #[serde(rename = "totalPages")]
    pub total_pages: i32,
    pub approximate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryAggregateDTO {
    #[serde(rename = "totalOrders")]
    pub total_orders: i64,
    #[serde(rename = "totalRevenue")]
    pub total_revenue: f64,
    #[serde(rename = "totalItems")]
    pub total_items: i64,
    #[serde(rename = "avgOrderValue")]
    pub avg_order_value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TotalsDTO {
    #[serde(rename = "totalOrders")]
    pub total_orders: i64,
    #[serde(rename = "totalRevenue")]
    pub total_revenue: f64,
    #[serde(rename = "totalItems")]
    pub total_items: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyAggregateDTO {
    pub date: String,
    pub categories: HashMap<String, CategoryAggregateDTO>,
    pub totals: TotalsDTO,
}

#[derive(Debug, Deserialize)]
pub struct CreateOrderItem {
    #[serde(rename = "productId")]
    pub product_id: i32,
    pub quantity: i32,
    #[serde(rename = "unitPrice")]
    pub unit_price: f64,
    #[serde(default)]
    pub discount: f64,
}

#[derive(Debug, Deserialize)]
pub struct CreateOrderRequest {
    #[serde(rename = "customerId")]
    pub customer_id: i32,
    #[serde(rename = "regionId")]
    pub region_id: i32,
    #[serde(default)]
    pub currency: String,
    #[serde(default)]
    pub notes: String,
    pub items: Vec<CreateOrderItem>,
}

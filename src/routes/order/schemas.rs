use std::collections::HashSet;

use crate::errors::GenericError;
use crate::routes::product::schemas::FulfillmentType;
use crate::routes::product::schemas::{CategoryDomain, PaymentType};
use crate::routes::user::schemas::DataSource;
use crate::schemas::{CountryCode, ONDCNetworkType};
// use crate::utils::deserialize_non_empty_vector;
use actix_http::Payload;
use actix_web::{web, FromRequest, HttpRequest};

use bigdecimal::BigDecimal;
use chrono::{DateTime, Utc};
use futures_util::future::LocalBoxFuture;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgHasArrayType;
use utoipa::ToSchema;
use uuid::Uuid;
#[derive(Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BuyerTerms {
    pub item_req: String,
    pub packaging_req: String,
}

#[derive(Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrderSelectItem {
    pub item_id: String,
    pub location_ids: Vec<String>,
    pub qty: i32,
    pub buyer_term: Option<BuyerTerms>,
    pub fulfillment_ids: Vec<String>,
}

#[derive(Deserialize, Debug, ToSchema, Serialize)]
pub struct Country {
    pub code: CountryCode,
    pub name: String,
}

#[derive(Deserialize, Debug, ToSchema, Serialize)]
pub struct City {
    pub code: String,
    pub name: String,
}

#[derive(Deserialize, Debug, ToSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FulfillmentLocation {
    pub gps: String,
    pub area_code: String,
    pub address: String,
    pub city: City,
    pub country: Country,
    pub state: String,
    pub contact_mobile_no: String,
}

#[derive(Deserialize, Debug, ToSchema, sqlx::Type)]
#[sqlx(type_name = "inco_term_type", rename_all = "UPPERCASE")]
#[serde(rename_all = "UPPERCASE")]
pub enum IncoTermType {
    Exw,
    Cif,
    Fob,
    Dap,
    Ddp,
}

impl PgHasArrayType for &IncoTermType {
    fn array_type_info() -> sqlx::postgres::PgTypeInfo {
        sqlx::postgres::PgTypeInfo::with_name("_inco_term_type")
    }
}

impl std::fmt::Display for IncoTermType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            IncoTermType::Exw => "EXW",
            IncoTermType::Cif => "CIF",
            IncoTermType::Fob => "FOB",
            IncoTermType::Dap => "DAP",
            IncoTermType::Ddp => "DDP",
        };

        write!(f, "{}", s)
    }
}

#[derive(Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrderDeliveyTerm {
    pub inco_terms: IncoTermType,
    pub place_of_delivery: String,
}

#[derive(Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrderSelectFulfillment {
    pub id: String,
    pub r#type: FulfillmentType,
    // #[serde(deserialize_with = "deserialize_non_empty_vector")]
    pub location: FulfillmentLocation,
    pub delivery_terms: Option<OrderDeliveyTerm>,
}

// #[derive(Deserialize, Debug, sqlx::Type)]
// #[sqlx(type_name = "commerce_data_type", rename_all = "snake_case")]
// #[serde(rename_all = "snake_case")]
// pub enum CommerceDataType {
//     Order,
//     PurchaseOrder,
// }

#[derive(Deserialize, Debug, ToSchema, PartialEq, sqlx::Type)]
#[sqlx(type_name = "commerce_data_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum OrderType {
    PurchaseOrder,
    SaleOrder,
}

#[derive(Debug, Serialize, Deserialize, sqlx::Type)]
#[sqlx(
    type_name = "fulfillment_servicability_status",
    rename_all = "snake_case"
)]
pub enum ServiceableType {
    #[serde(rename = "non_serviceable")]
    NonServiceable,
    #[serde(rename = "serviceable")]
    Serviceable,
}
impl PgHasArrayType for ServiceableType {
    fn array_type_info() -> sqlx::postgres::PgTypeInfo {
        sqlx::postgres::PgTypeInfo::with_name("_fulfillment_servicability_status")
    }
}

#[derive(Debug, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "fulfillment_category_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum FulfillmentCategoryType {
    #[serde(rename = "standard_delivery")]
    StandardDelivery,
    #[serde(rename = "express_delivery")]
    ExpressDelivery,
    #[serde(rename = "self_pickup")]
    SelfPickup,
}
impl PgHasArrayType for FulfillmentCategoryType {
    fn array_type_info() -> sqlx::postgres::PgTypeInfo {
        sqlx::postgres::PgTypeInfo::with_name("_fulfillment_category_type")
    }
}

// impl std::fmt::Display for FulfillmentCategoryType {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         let s = match self {
//             FulfillmentCategoryType::StandardDelivery => "standard_delivery",
//             FulfillmentCategoryType::ExpressDelivery => "express_delivery",
//             FulfillmentCategoryType::SelfPickup => "self_pickup",
//         };

//         write!(f, "{}", s)
//     }
// }

#[derive(Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrderSelectRequest {
    #[schema(value_type = String)]
    pub transaction_id: Uuid,
    #[schema(value_type = String)]
    pub message_id: Uuid,
    pub domain_category_code: CategoryDomain,
    pub payment_types: Vec<PaymentType>,
    pub provider_id: String,
    pub items: Vec<OrderSelectItem>,
    pub ttl: String,
    pub fulfillments: Vec<OrderSelectFulfillment>,
    pub order_type: OrderType,
    pub bpp_id: String,
    pub is_import: bool,
}

impl FromRequest for OrderSelectRequest {
    type Error = GenericError;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let fut = web::Json::<Self>::from_request(req, payload);

        Box::pin(async move {
            match fut.await {
                Ok(json) => Ok(json.into_inner()),
                Err(e) => Err(GenericError::ValidationError(e.to_string())),
            }
        })
    }
}

#[derive(Deserialize, Debug, sqlx::Type)]
#[sqlx(type_name = "buyer_commerce_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum CommerceStatusType {
    QuoteRequested,
    QuoteAccepted,
    QuoteRejected,
    Initialized,
    Created,
    Accepted,
    InProgress,
    Completed,
    Cancelled,
}

// #[derive(Deserialize, Debug)]
// pub struct OrderStatusHistory {
//     created_on: DateTime<Utc>,
//     status: CommerceStatusType,
// }

#[derive(Deserialize, Debug, Serialize, sqlx::Encode)]
#[serde(rename_all = "camelCase")]
pub struct Payment {
    pub collected_by: Option<ONDCNetworkType>,
    pub r#type: PaymentType,
}

#[derive(Deserialize, Debug, Serialize)]
pub struct DropOffLocation {
    pub gps: String,
    pub area_code: String,
    pub address: Option<String>,
    pub city: String,
    pub country: CountryCode,
    pub state: String,
}
#[derive(Deserialize, Debug, Serialize)]
pub struct DropOffContact {
    pub mobile_no: String,
    pub email: Option<String>,
}

#[derive(Deserialize, Debug, Serialize)]
pub struct DropOffData {
    pub location: DropOffLocation,
    pub contact: DropOffContact,
}

#[derive(Deserialize, Debug, Serialize)]
pub struct PickUpData {
    pub location: DropOffLocation,
    pub contact: DropOffContact,
}

#[derive(Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrderInitBilling {
    pub name: String,
    pub address: String,
    pub tax_id: String,
    pub mobile_no: String,
    pub email: String,
    pub city: City,
    pub state: String,
}

#[derive(Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OrderInitRequest {
    #[schema(value_type = String)]
    pub transaction_id: Uuid,
    #[schema(value_type = String)]
    pub message_id: Uuid,
    pub billing: OrderInitBilling,
}

impl FromRequest for OrderInitRequest {
    type Error = GenericError;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let fut = web::Json::<Self>::from_request(req, payload);

        Box::pin(async move {
            match fut.await {
                Ok(json) => Ok(json.into_inner()),
                Err(e) => Err(GenericError::ValidationError(e.to_string())),
            }
        })
    }
}

#[derive(Debug)]
pub struct BuyerCommerceSeller {
    pub id: String,
    pub name: String,
}

#[derive(Debug)]
pub struct BasicNetWorkData {
    pub id: String,
    pub uri: String,
}
#[derive(Debug)]
pub struct BuyerCommercePayment {
    pub id: Uuid,
    pub collected_by: Option<ONDCNetworkType>,
    pub payment_type: PaymentType,
}

#[derive(Debug, sqlx::Type)]
#[sqlx(type_name = "commerce_fulfillment_status_type")]
#[sqlx(rename_all = "snake_case")]
pub enum CommerceFulfillmentStatusType {
    AgentAssigned,
    Packed,
    OutForDelivery,
    OrderPickedUp,
    SearchingForAgent,
    Pending,
    OrderDelivered,
    Cancelled,
}

#[derive(Debug)]
pub struct DeliveryTerm {
    pub inco_terms: IncoTermType,
    pub place_of_delivery: String,
}
#[derive(Debug)]
pub struct BuyerCommerceFulfillment {
    pub id: String,
    pub fulfillment_id: String,
    pub fulfillment_type: FulfillmentType,
    pub tat: Option<String>,
    pub fulfillment_status: CommerceFulfillmentStatusType,
    pub delivery_term: Option<DeliveryTerm>,
    pub provider_name: Option<String>,
    pub category: FulfillmentCategoryType,
    pub servicable_status: Option<ServiceableType>,
    pub drop_off: Option<DropOffData>,
    pub pickup: Option<PickUpData>,
    tracking: Option<bool>,
}

#[derive(Debug)]
pub struct BuyerTerm {
    pub item_req: String,
    pub packaging_req: String,
}

#[derive(Debug)]
pub struct BuyerCommerceItem {
    pub id: Uuid,
    pub item_id: String,
    pub item_name: String,
    pub item_code: String,
    pub item_image: String,
    pub qty: BigDecimal,
    pub buyer_terms: Option<BuyerTerm>,
    pub tax_rate: BigDecimal,
    pub tax_value: BigDecimal,
    pub unit_price: BigDecimal,
    pub gross_total: BigDecimal,
    pub available_qty: BigDecimal,
    pub discount_amount: BigDecimal,
    pub location_ids: Vec<String>,
    pub fulfillment_ids: Vec<String>,
}

#[derive(Debug)]
pub struct BuyerCommerce {
    pub id: Uuid,
    pub urn: Option<Uuid>,
    pub external_urn: Uuid,
    pub record_type: OrderType,
    pub record_status: CommerceStatusType,
    pub domain_category_code: CategoryDomain,
    pub seller: BuyerCommerceSeller,
    pub source: DataSource,
    pub created_on: DateTime<Utc>,
    pub updated_on: Option<DateTime<Utc>>,
    pub created_by: Uuid,
    pub grand_total: Option<BigDecimal>,
    pub bap: BasicNetWorkData,
    pub bpp: BasicNetWorkData,
    pub is_import: bool,
    pub quote_ttl: String,
    pub city_code: String,
    pub country_code: CountryCode,
    pub items: Vec<BuyerCommerceItem>,
    pub payments: Vec<BuyerCommercePayment>,
    pub fulfillments: Vec<BuyerCommerceFulfillment>,
}

impl BuyerCommerce {
    pub fn get_ondc_location_ids(&self) -> Vec<&str> {
        let mut unique_ids = HashSet::new();

        self.items
            .iter()
            .flat_map(|item| item.location_ids.iter().map(|id| id.as_str()))
            .filter(|id| unique_ids.insert(*id))
            .collect()
    }
}

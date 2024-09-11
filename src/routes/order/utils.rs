use super::schemas::{
    BPPTermsModel, BasicNetWorkData, BuyerCommerce, BuyerCommerceDataModel,
    BuyerCommerceFulfillment, BuyerCommerceFulfillmentModel, BuyerCommerceItem,
    BuyerCommerceItemModel, BuyerCommercePayment, BuyerCommercePaymentModel, BuyerCommerceSeller,
    BuyerTerm, DropOffContactModel, DropOffData, DropOffDataModel, DropOffLocationModel,
    FulfillmentContact, FulfillmentLocation, OrderBillingModel, OrderCancellationFeeModel,
    OrderCancellationTermModel, OrderSelectFulfillment, OrderSelectRequest,
    PaymentSettlementDetailModel, PickUpData, PickUpDataModel, SelectFulfillmentLocation,
};
use crate::constants::ONDC_TTL;
use crate::routes::ondc::buyer::schemas::{
    BreakupTitleType, ONDCBilling, ONDCBreakUp, ONDCFulfillment, ONDCFulfillmentCategoryType,
    ONDCFulfillmentStopType, ONDCOnInitPayment, ONDCOnInitRequest, ONDCOnSelectFulfillment,
    ONDCOnSelectPayment, ONDCOnSelectRequest, ONDCOrderCancellationTerm, ONDCOrderFulfillmentEnd,
    ONDCSelectFulfillmentLocation, ONDCSelectRequest, ONDCTag, ONDCTagItemCode, ONDCTagType,
    SellerProductInfo, SettlementBasis, TagTrait,
};
use crate::routes::ondc::buyer::utils::{
    get_ondc_seller_mapping_key, get_ondc_seller_product_info_mapping, get_tag_value_from_list,
};
use crate::routes::ondc::{LookupData, ONDCActionType};
use crate::routes::order::schemas::{
    CommerceFulfillmentStatusType, CommerceStatusType, DeliveryTerm, FulfillmentCategoryType,
    IncoTermType, OrderType, ServiceableType,
};
use crate::routes::product::schemas::{CategoryDomain, FulfillmentType, PaymentType};
use crate::routes::user::schemas::{BusinessAccount, DataSource, UserAccount};
use crate::schemas::{
    CountryCode, CurrencyType, FeeType, ONDCNetworkType, RegisteredNetworkParticipant,
    RequestMetaData,
};
use anyhow::Context;
use bigdecimal::BigDecimal;
use chrono::Utc;
use serde_json::Value;
use sqlx::types::Json;
use sqlx::{Executor, PgPool, Postgres, Transaction};
use std::collections::HashMap;
use std::str::FromStr;
use uuid::Uuid;
use validator::HasLen;

#[allow(clippy::too_many_arguments)]
#[tracing::instrument(name = "Save Product Search Request", skip(pool))]
pub async fn save_ondc_order_request(
    pool: &PgPool,
    user_account: &UserAccount,
    business_account: &BusinessAccount,
    meta_data: &RequestMetaData,
    request_payload: &Value,
    transaction_id: Uuid,
    message_id: Uuid,
    action_type: ONDCActionType,
) -> Result<(), anyhow::Error> {
    sqlx::query!(
        r#"
        INSERT INTO ondc_buyer_order_req (message_id, transaction_id, device_id,  user_id, business_id, action_type, request_payload)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
        &message_id,
        &transaction_id,
        &meta_data.device_id,
        &user_account.id,
        &business_account.id,
        &action_type.to_string(),
        &request_payload

    )
    .execute(pool).await
    .map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e).context("A database failure occurred while saving ONDC order request")
    })?;
    Ok(())
}

#[tracing::instrument(name = "save rfq", skip(transaction))]
pub async fn save_rfq_order(
    transaction: &mut Transaction<'_, Postgres>,
    select_request: &OrderSelectRequest,
    user_account: &UserAccount,
    business_account: &BusinessAccount,
    bpp_detail: &LookupData,
    bap_detail: &RegisteredNetworkParticipant,
    provider_name: &str,
) -> Result<Uuid, anyhow::Error> {
    let order_id = Uuid::new_v4();
    let query = sqlx::query!(
        r#"
        INSERT INTO buyer_commerce_data (id, external_urn, record_type, record_status, 
        domain_category_code, buyer_id, seller_id, seller_name, buyer_name, source, created_on, created_by, bpp_id, bpp_uri, bap_id, bap_uri, is_import, quote_ttl, city_code, country_code)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)
        ON CONFLICT (external_urn) 
        DO NOTHING
        "#,
        order_id,
        &select_request.transaction_id,
        &select_request.order_type as &OrderType,
        CommerceStatusType::QuoteRequested as CommerceStatusType,
        &select_request.domain_category_code as &CategoryDomain,
        &business_account.id,
        &select_request.provider_id,
        &provider_name,
        &business_account.company_name,
        DataSource::PlaceOrder as DataSource,
        Utc::now(),
        &user_account.id,
        &select_request.bpp_id,
        bpp_detail.subscriber_url,
        &bap_detail.subscriber_id,
        &bap_detail.subscriber_uri,
        &select_request.is_import,
        &select_request.ttl,
        &select_request.fulfillments[0].location.city.code,
        &select_request.fulfillments[0].location.country.code as &CountryCode,
    );

    transaction.execute(query).await.map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e)
            .context("A database failure occurred while saving RFQ to database request")
    })?;
    Ok(order_id)
}

#[tracing::instrument(name = "delete  fulfillment", skip(transaction))]
pub async fn delete_fulfillment_by_order_id(
    transaction: &mut Transaction<'_, Postgres>,
    order_id: &Uuid,
) -> Result<(), anyhow::Error> {
    let query = sqlx::query(
        r#"
        DELETE FROM buyer_commerce_fulfillment_data
        WHERE commerce_data_id = $1
        "#,
    )
    .bind(order_id);

    transaction
        .execute(query) // Dereference the transaction
        .await
        .map_err(|e| {
            tracing::error!("Failed to execute delete query: {:?}", e);
            anyhow::Error::new(e)
                .context("A database failure occurred while deleting the order fulfillment")
        })?;

    Ok(())
}

pub fn create_drop_off_from_rfq_select_fulfullment(
    fulfillment: &SelectFulfillmentLocation,
) -> DropOffDataModel {
    // let mut drop_list = vec![];
    // for fulfillment in select_fulfillments {
    DropOffDataModel {
        location: DropOffLocationModel {
            gps: fulfillment.gps.clone(),
            area_code: fulfillment.area_code.clone(),
            address: Some(fulfillment.address.clone()),
            city: fulfillment.city.name.clone(),
            country: fulfillment.country.code.clone(),
            state: fulfillment.state.clone(),
        },
        contact: DropOffContactModel {
            mobile_no: fulfillment.contact_mobile_no.clone(),
            email: None,
        },
    }
}

#[tracing::instrument(name = "save rfq fulfillment", skip(transaction))]
pub async fn save_rfq_fulfillment(
    transaction: &mut Transaction<'_, Postgres>,
    order_id: &Uuid,
    fulfillments: &Vec<OrderSelectFulfillment>,
) -> Result<(), anyhow::Error> {
    // delete_fulfillment_by_order_id(transaction, order_id).await?;
    let mut id_list = vec![];
    let mut order_list = vec![];
    let mut fulfillment_id_list = vec![];
    let mut fulfillment_type_list = vec![];
    let mut drop_off_data_list = vec![];
    let mut incoterms_list = vec![];
    let mut delivery_place_list = vec![];
    for fulfillment in fulfillments {
        order_list.push(*order_id);
        id_list.push(Uuid::new_v4());
        fulfillment_id_list.push(fulfillment.id.as_str());
        fulfillment_type_list.push(&fulfillment.r#type);
        if fulfillment.r#type == FulfillmentType::Delivery {
            drop_off_data_list.push(
                serde_json::to_value(&create_drop_off_from_rfq_select_fulfullment(
                    &fulfillment.location,
                ))
                .unwrap(),
            );
        }

        incoterms_list.push(fulfillment.delivery_terms.as_ref().map(|e| &e.inco_terms));
        delivery_place_list.push(
            fulfillment
                .delivery_terms
                .as_ref()
                .map(|e| e.place_of_delivery.as_str()),
        );
    }
    let query = sqlx::query!(
        r#"
        INSERT INTO buyer_commerce_fulfillment_data (id, commerce_data_id, fulfillment_id, fulfillment_type, inco_terms, place_of_delivery, drop_off_data)
        SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::text[], $4::fulfillment_type[],$5::inco_term_type [], $6::text[], $7::jsonb[]);
        "#,
        &id_list[..] as &[Uuid],
        &order_list[..] as &[Uuid],
        &fulfillment_id_list[..] as &[&str],
        &fulfillment_type_list[..] as &[&FulfillmentType],
        &incoterms_list[..] as &[Option<&IncoTermType>],
        &delivery_place_list[..] as &[Option<&str>],
        &drop_off_data_list[..] as &[Value]
    );

    transaction.execute(query).await.map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e)
            .context("A database failure occurred while saving RFQ to database request")
    })?;
    Ok(())
}

#[tracing::instrument(name = "save rfq items", skip(transaction))]
pub async fn save_order_select_items(
    transaction: &mut Transaction<'_, Postgres>,
    order_id: &Uuid,
    select_request: &OrderSelectRequest,
    product_map: &HashMap<String, SellerProductInfo>,
) -> Result<(), anyhow::Error> {
    let item_count = select_request.items.length();
    let line_id_list: Vec<Uuid> = (0..item_count).map(|_| Uuid::new_v4()).collect();
    let order_id_list: Vec<Uuid> = vec![*order_id; item_count as usize];
    let mut item_id_list = vec![];
    let mut item_code_list: Vec<Option<&str>> = vec![];
    let mut item_name_list = vec![];
    let mut location_id_list = vec![];
    let mut fulfillment_id_list = vec![];
    let mut item_image_list = vec![];
    let mut qty_list = vec![];
    let mut mrp_list = vec![];
    let mut unit_price_list = vec![];
    let mut tax_rate_list = vec![];
    let mut item_req_list = vec![];
    let mut packagin_req_list = vec![];
    for item in &select_request.items {
        let key = get_ondc_seller_mapping_key(
            &select_request.bpp_id,
            &select_request.provider_id,
            &item.item_id,
        );
        if let Some(seller_item_obj) = product_map.get(&key) {
            item_code_list.push(seller_item_obj.item_code.as_deref());
            item_name_list.push(seller_item_obj.item_name.as_str());
            item_image_list.push(
                seller_item_obj
                    .images
                    .as_array()
                    .and_then(|images| images.first())
                    .and_then(|image| image.as_str())
                    .unwrap_or(""),
            );
            mrp_list.push(seller_item_obj.mrp.clone());
            unit_price_list.push(seller_item_obj.unit_price.clone());
            tax_rate_list.push(seller_item_obj.tax_rate.clone());
        } else {
            item_code_list.push(None);
            item_name_list.push("");
            item_image_list.push("");
            mrp_list.push(BigDecimal::from(0));
            unit_price_list.push(BigDecimal::from(0));
            tax_rate_list.push(BigDecimal::from(0));
        }
        // let item_name = '';
        // let item_image = ''.as_str();
        item_id_list.push(item.item_id.as_str());

        location_id_list.push(serde_json::to_value(&item.location_ids)?); // Serialize to JSON
        fulfillment_id_list.push(serde_json::to_value(&item.fulfillment_ids)?);

        qty_list.push(BigDecimal::from(item.qty));
        item_req_list.push(item.buyer_term.as_ref().map(|e| e.item_req.as_str()));
        packagin_req_list.push(item.buyer_term.as_ref().map(|e| e.packaging_req.as_str()));
    }
    let query = sqlx::query!(
        r#"
        INSERT INTO buyer_commerce_data_line (id, commerce_data_id, item_id, item_name, item_code, item_image, 
            qty, location_ids, fulfillment_ids, tax_rate, mrp, unit_price, item_req, packaging_req)
            SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::text[], $4::text[], $5::text[], $6::text[],
             $7::decimal[], $8::jsonb[], $9::jsonb[], $10::decimal[], $11::decimal[], $12::decimal[], $13::text[], $14::text[])
        ON CONFLICT (commerce_data_id, item_code) 
        DO NOTHING
        "#,
        &line_id_list[..] as &[Uuid],
        &order_id_list[..] as &[Uuid],
        &item_id_list[..] as &[&str],
        &item_name_list[..] as &[&str],
        &item_code_list[..] as &[Option<&str>], //change
        &item_image_list[..] as &[&str],        //change
        &qty_list[..] as &[BigDecimal],
        &location_id_list as &[Value],
        &fulfillment_id_list as &[Value],
        &tax_rate_list[..] as &[BigDecimal],
        &mrp_list[..] as &[BigDecimal],
        &unit_price_list[..] as &[BigDecimal],
        &item_req_list[..] as &[Option<&str>],
        &packagin_req_list[..] as &[Option<&str>],
    );

    transaction.execute(query).await.map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e)
            .context("A database failure occurred while saving RFQ to database request")
    })?;
    Ok(())
}

#[tracing::instrument(name = "delete order", skip(transaction))]
pub async fn delete_order(
    transaction: &mut Transaction<'_, Postgres>,
    id: &Uuid,
) -> Result<(), anyhow::Error> {
    let query = sqlx::query(
        r#"
        DELETE FROM buyer_commerce_data
        WHERE external_urn = $1
        "#,
    )
    .bind(id);

    transaction
        .execute(query) // Dereference the transaction
        .await
        .map_err(|e| {
            tracing::error!("Failed to execute delete query: {:?}", e);
            anyhow::Error::new(e).context("A database failure occurred while deleting the order")
        })?;

    Ok(())
}

#[tracing::instrument(name = "save on select payments", skip(transaction))]
pub async fn save_payment_obj_select(
    transaction: &mut Transaction<'_, Postgres>,
    order_id: &Uuid,
    payments: &Vec<PaymentType>,
) -> Result<(), anyhow::Error> {
    // delete_on_select_payment(transaction, order_id).await?;
    let mut id_list = vec![];
    let mut commerce_data_id_list = vec![];
    for _ in 0..payments.len() {
        id_list.push(Uuid::new_v4());
        commerce_data_id_list.push(*order_id);
    }
    let query = sqlx::query!(
        r#"
        INSERT INTO buyer_commerce_payment(id, commerce_data_id, payment_type)
            SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::payment_type[])
        "#,
        &id_list[..] as &[Uuid],
        &commerce_data_id_list[..] as &[Uuid],
        &payments[..] as &[PaymentType]
    );

    transaction.execute(query).await.map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e)
            .context("A database failure occurred while saving select payment to database request")
    })?;
    Ok(())
}

#[tracing::instrument(name = "save request for quote", skip(pool))]
pub async fn initialize_order_select(
    pool: &PgPool,
    user_account: &UserAccount,
    business_account: &BusinessAccount,
    select_request: &OrderSelectRequest,
    bap_detail: &RegisteredNetworkParticipant,
    bpp_detail: &LookupData,
) -> Result<(), anyhow::Error> {
    let item_code_list: Vec<&str> = select_request
        .items
        .iter()
        .map(|item| item.item_id.as_str()) // Assuming item_id is a String
        .collect();
    let seller_product_map = get_ondc_seller_product_info_mapping(
        pool,
        &bpp_detail.subscriber_id,
        &select_request.provider_id,
        &item_code_list,
    )
    .await?;
    let provider_name = seller_product_map
        .values()
        .next()
        .and_then(|obj| obj.provider_name.as_deref())
        .unwrap_or_default();
    let mut transaction = pool
        .begin()
        .await
        .context("Failed to acquire a Postgres connection from the pool")?;
    delete_order(&mut transaction, &select_request.transaction_id).await?;

    let order_id = save_rfq_order(
        &mut transaction,
        select_request,
        user_account,
        business_account,
        bpp_detail,
        bap_detail,
        provider_name,
    )
    .await?;
    save_rfq_fulfillment(&mut transaction, &order_id, &select_request.fulfillments).await?;

    save_order_select_items(
        &mut transaction,
        &order_id,
        select_request,
        &seller_product_map,
    )
    .await?;

    save_payment_obj_select(&mut transaction, &order_id, &select_request.payment_types).await?;

    transaction
        .commit()
        .await
        .context("Failed to commit SQL transaction to store a order")?;

    Ok(())
}

pub fn create_drop_off_from_ondc_select_fulfullment(
    ondc_select_fulfillment: &[ONDCFulfillment<ONDCSelectFulfillmentLocation>],
    // contact: &ONDCContact,
) -> Option<DropOffDataModel> {
    if let Some(stops) = &ondc_select_fulfillment[0].stops {
        let location = &stops[0].location;
        let contact = &stops[0].contact;
        Some(DropOffDataModel {
            location: DropOffLocationModel {
                gps: location.gps.clone(),
                area_code: location.area_code.clone(),
                address: location.address.clone(),
                city: location.city.name.clone(),
                country: location.country.code.clone(),
                state: location.state.name.clone(),
            },
            contact: DropOffContactModel {
                mobile_no: contact.phone.clone(),
                email: contact.email.clone(),
            },
        })
    } else {
        None
    }
}

pub fn create_pick_off_from_ondc_select_fulfillment(
    ondc_select_fulfillment_ends: &Option<
        Vec<ONDCOrderFulfillmentEnd<ONDCSelectFulfillmentLocation>>,
    >,
    // contact: &ONDCContact,
) -> Option<PickUpDataModel> {
    if let Some(ondc_select_fulfillment_end_res) = ondc_select_fulfillment_ends {
        for ondc_select_fulfillment_end in ondc_select_fulfillment_end_res {
            if ondc_select_fulfillment_end.r#type == ONDCFulfillmentStopType::Start {
                return Some(PickUpDataModel {
                    location: DropOffLocationModel {
                        gps: ondc_select_fulfillment_end.location.gps.clone(),
                        area_code: ondc_select_fulfillment_end.location.area_code.clone(),
                        address: ondc_select_fulfillment_end.location.address.clone(),
                        city: ondc_select_fulfillment_end.location.city.name.clone(),
                        country: ondc_select_fulfillment_end.location.country.code.clone(),
                        state: ondc_select_fulfillment_end.location.state.name.clone(),
                    },
                    contact: DropOffContactModel {
                        mobile_no: ondc_select_fulfillment_end.contact.phone.clone(),
                        email: ondc_select_fulfillment_end.contact.email.clone(),
                    },
                });
            }
        }
    }
    None
}

#[tracing::instrument(name = "save on select fulfillment", skip(transaction))]
pub async fn save_on_select_fulfillment(
    transaction: &mut Transaction<'_, Postgres>,
    order_id: &Uuid,
    select_fulfillment: &Vec<ONDCFulfillment<ONDCSelectFulfillmentLocation>>,
    on_select_fulfillments: &Vec<ONDCOnSelectFulfillment>,
) -> Result<(), anyhow::Error> {
    // delete_fulfillment_by_order_id(transaction, order_id).await?;
    let mut id_list = vec![];
    let mut order_list = vec![];
    let mut fulfillment_id_list = vec![];
    let mut fulfillment_type_list = vec![];
    let mut drop_off_data_list = vec![];
    let mut incoterms_list = vec![];
    let mut delivery_place_list = vec![];
    let mut provider_name_list = vec![];
    let mut tat_list = vec![];
    let mut category_list = vec![];
    let mut servicable_status_list = vec![];
    let mut tracking_list = vec![];
    let drop_off_data = create_drop_off_from_ondc_select_fulfullment(select_fulfillment);
    let drop_off_data_json = serde_json::to_value(drop_off_data).unwrap_or_default();
    let mut pickup_data_list = vec![];

    for fulfillment in on_select_fulfillments {
        order_list.push(*order_id);
        id_list.push(Uuid::new_v4());
        fulfillment_id_list.push(fulfillment.id.as_str());

        fulfillment_type_list.push(
            if fulfillment.category == ONDCFulfillmentCategoryType::SelfPickup {
                &FulfillmentType::SelfPickup
            } else {
                &FulfillmentType::Delivery
            },
        );
        incoterms_list.push(select_fulfillment[0].tags.as_ref().map(|e| {
            e[0].get_tag_value(&ONDCTagItemCode::IncoTerms.to_string())
                .unwrap_or("")
        }));
        delivery_place_list.push(select_fulfillment[0].tags.as_ref().map(|e| {
            e[0].get_tag_value(&ONDCTagItemCode::NamedPlaceOfDelivery.to_string())
                .unwrap_or("")
        }));
        provider_name_list.push(fulfillment.provider_name.as_deref());
        tat_list.push(fulfillment.tat.as_str());
        category_list.push(fulfillment.category.get_category_type());
        servicable_status_list.push(fulfillment.state.descriptor.code.get_servicable_type());
        tracking_list.push(fulfillment.tracking);
        drop_off_data_list.push(drop_off_data_json.clone());
        pickup_data_list.push(
            serde_json::to_value(create_pick_off_from_ondc_select_fulfillment(
                &fulfillment.stops,
            ))
            .unwrap_or_default(),
        );
    }
    let query = sqlx::query!(
        r#"
        INSERT INTO buyer_commerce_fulfillment_data (id, commerce_data_id, fulfillment_id, fulfillment_type, inco_terms, 
            place_of_delivery, drop_off_data, pickup_data, provider_name, servicable_status, tracking, tat, category)
        SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::text[], $4::fulfillment_type[],$5::inco_term_type [], $6::text[],
             $7::jsonb[], $8::jsonb[], $9::text[], $10::fulfillment_servicability_status[], $11::bool[], $12::text[], $13::fulfillment_category_type[]);
        "#,
        &id_list[..] as &[Uuid],
        &order_list[..] as &[Uuid],
        &fulfillment_id_list[..] as &[&str],
        &fulfillment_type_list[..] as &[&FulfillmentType],
        &incoterms_list[..] as &[Option<&str>],
        &delivery_place_list[..] as &[Option<&str>],
        &drop_off_data_list[..] as &[Value],
        &pickup_data_list[..] as &[Value],
        &provider_name_list[..] as &[Option<&str>],
        &servicable_status_list[..] as &[ServiceableType],
        &tracking_list[..] as &[bool],
        &tat_list[..] as &[&str],
        &category_list[..] as &[FulfillmentCategoryType],
    );

    transaction.execute(query).await.map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e)
            .context("A database failure occurred while saving RFQ to database request")
    })?;
    Ok(())
}

#[tracing::instrument(name = "save order on select", skip(transaction))]
pub async fn save_buyer_order_data_on_select(
    transaction: &mut Transaction<'_, Postgres>,
    ondc_select_req: &ONDCSelectRequest,
    ondc_on_select_req: &ONDCOnSelectRequest,
    user_account: &UserAccount,
    business_account: &BusinessAccount,
    provider_name: &str,
) -> Result<Uuid, anyhow::Error> {
    let grand_total =
        BigDecimal::from_str(&ondc_on_select_req.message.order.quote.price.value).unwrap();
    let order_id = Uuid::new_v4();
    let is_import = ondc_select_req.message.order.fulfillments[0].tags.is_some();
    let mut created_on = ondc_on_select_req.context.timestamp;
    let mut order_type = OrderType::SaleOrder;
    if ondc_select_req.context.ttl != ONDC_TTL {
        order_type = OrderType::PurchaseOrder;
        created_on = ondc_select_req.context.timestamp;
    };
    let order_status = if ondc_on_select_req.error.is_none() {
        CommerceStatusType::QuoteAccepted
    } else {
        CommerceStatusType::QuoteRejected
    };
    let query = sqlx::query!(
        r#"
        INSERT INTO buyer_commerce_data (id, external_urn, record_type, record_status,
        domain_category_code, buyer_id, seller_id, seller_name, buyer_name, source, created_on, created_by, bpp_id, bpp_uri,
        bap_id, bap_uri, is_import, quote_ttl, updated_on, currency_code, grand_total, city_code, country_code)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23)
        ON CONFLICT (external_urn)
        DO UPDATE SET
        record_status = EXCLUDED.record_status,
        updated_on = EXCLUDED.updated_on,
        currency_code = EXCLUDED.currency_code
        RETURNING id
        "#,
        order_id,
        &ondc_on_select_req.context.transaction_id,
        &order_type as &OrderType,
        &order_status as &CommerceStatusType,
        &ondc_on_select_req.context.domain.get_category_domain() as &CategoryDomain,
        &business_account.id,
        &ondc_on_select_req.message.order.provider.id,
        &provider_name,
        &business_account.company_name,
        DataSource::PlaceOrder as DataSource,
        created_on,
        &user_account.id,
        ondc_on_select_req.context.bpp_id.as_deref().unwrap_or(""),
        ondc_on_select_req.context.bpp_uri.as_deref().unwrap_or(""),
        &ondc_on_select_req.context.bap_id,
        &ondc_on_select_req.context.bap_uri,
        is_import,
        &ondc_select_req.context.ttl,
        ondc_select_req.context.timestamp,
        &ondc_on_select_req.message.order.quote.price.currency as &CurrencyType,
        &grand_total,
        &ondc_select_req.context.location.city.code,
        &ondc_select_req.context.location.country.code as &CountryCode,
    );

    let result = query.fetch_one(&mut **transaction).await.map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e)
            .context("A database failure occurred while saving RFQ to database request")
    })?;
    Ok(result.id)
}

#[tracing::instrument(name = "save order on on_select", skip(pool))]
pub async fn initialize_order_on_select(
    pool: &PgPool,
    on_select_request: &ONDCOnSelectRequest,
    user_account: &UserAccount,
    business_account: &BusinessAccount,
    ondc_select_req: &ONDCSelectRequest,
) -> Result<(), anyhow::Error> {
    let bpp_id = on_select_request.context.bpp_id.as_deref().unwrap_or("");
    let item_code_list: Vec<&str> = on_select_request
        .message
        .order
        .items
        .iter()
        .map(|item| item.id.as_str()) // Assuming item_id is a String
        .collect();
    let seller_product_map = get_ondc_seller_product_info_mapping(
        pool,
        bpp_id,
        &on_select_request.message.order.provider.id,
        &item_code_list,
    )
    .await?;
    let provider_name = seller_product_map
        .values()
        .next()
        .and_then(|obj| obj.provider_name.as_deref())
        .unwrap_or_default();
    let mut transaction = pool
        .begin()
        .await
        .context("Failed to acquire a Postgres connection from the pool")?;

    delete_order(&mut transaction, &ondc_select_req.context.transaction_id).await?;

    let order_id = save_buyer_order_data_on_select(
        &mut transaction,
        ondc_select_req,
        on_select_request,
        user_account,
        business_account,
        provider_name,
    )
    .await?;

    let _ = save_order_on_select_items(
        &mut transaction,
        &order_id,
        on_select_request,
        &seller_product_map,
    )
    .await;

    save_payment_obj_on_select(
        &mut transaction,
        &order_id,
        &on_select_request.message.order.payments,
    )
    .await?;
    save_on_select_fulfillment(
        &mut transaction,
        &order_id,
        &ondc_select_req.message.order.fulfillments,
        &on_select_request.message.order.fulfillments,
    )
    .await?;
    transaction
        .commit()
        .await
        .context("Failed to commit SQL transaction to store an order")?;

    Ok(())
}

pub fn get_quote_item_value_mapping<'a>(
    breakups: &'a Vec<ONDCBreakUp>,
    title_type: &BreakupTitleType,
) -> HashMap<&'a String, BigDecimal> {
    let mut header_map = HashMap::new();
    for breakup in breakups {
        if &breakup.title_type == title_type {
            if let Some(item_id) = &breakup.item_id {
                let break_up_value = BigDecimal::from_str(&breakup.price.value)
                    .unwrap_or_else(|_| BigDecimal::from(0));
                header_map.insert(item_id, break_up_value);
            }
        }
    }
    header_map
}

pub fn get_quote_item_breakup_mapping<'a>(
    breakups: &'a Vec<ONDCBreakUp>,
    title_type: &BreakupTitleType,
) -> HashMap<&'a String, &'a ONDCBreakUp> {
    let mut header_map = HashMap::new();
    for breakup in breakups {
        if &breakup.title_type == title_type {
            if let Some(item_id) = &breakup.item_id {
                header_map.insert(item_id, breakup);
            }
        }
    }
    header_map
}

#[tracing::instrument(name = "delete on select payment", skip(transaction))]
pub async fn delete_on_select_payment(
    transaction: &mut Transaction<'_, Postgres>,
    id: &Uuid,
) -> Result<(), anyhow::Error> {
    let query = sqlx::query(
        r#"
        DELETE FROM buyer_commerce_payment
        WHERE commerce_data_id = $1
        "#,
    )
    .bind(id);

    transaction
        .execute(query) // Dereference the transaction
        .await
        .map_err(|e| {
            tracing::error!("Failed to execute delete query: {:?}", e);
            anyhow::Error::new(e)
                .context("A database failure occurred while deleting the on select payment")
        })?;

    Ok(())
}

#[tracing::instrument(name = "save on select payments", skip(transaction))]
pub async fn save_payment_obj_on_select(
    transaction: &mut Transaction<'_, Postgres>,
    order_id: &Uuid,
    payments: &Vec<ONDCOnSelectPayment>,
) -> Result<(), anyhow::Error> {
    // delete_on_select_payment(transaction, order_id).await?;
    let mut id_list = vec![];
    let mut commerce_data_id_list = vec![];
    let mut collected_by_list = vec![];
    let mut payment_type_list = vec![];
    for payment in payments {
        id_list.push(Uuid::new_v4());
        commerce_data_id_list.push(*order_id);
        collected_by_list.push(payment.collected_by.clone());
        payment_type_list.push(payment.r#type.get_payment());
    }
    let query = sqlx::query!(
        r#"
        INSERT INTO buyer_commerce_payment(id, commerce_data_id, collected_by, payment_type)
            SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::ondc_network_participant_type[],  $4::payment_type[])
        "#,
        &id_list[..] as &[Uuid],
        &commerce_data_id_list[..] as &[Uuid],
        &collected_by_list[..] as &[ONDCNetworkType],
        &payment_type_list[..] as &[PaymentType]
    );

    transaction.execute(query).await.map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e)
            .context("A database failure occurred while saving RFQ to database request")
    })?;
    Ok(())
}

#[tracing::instrument(name = "save on select items", skip(transaction))]
pub async fn save_order_on_select_items(
    transaction: &mut Transaction<'_, Postgres>,
    order_id: &Uuid,
    ondc_on_select_request: &ONDCOnSelectRequest,
    product_map: &HashMap<String, SellerProductInfo>,
) -> Result<(), anyhow::Error> {
    let item_count = ondc_on_select_request.message.order.items.length();
    let line_id_list: Vec<Uuid> = (0..item_count).map(|_| Uuid::new_v4()).collect();
    let order_id_list: Vec<Uuid> = vec![*order_id; item_count as usize];
    let mut item_id_list = vec![];
    let mut item_code_list: Vec<Option<&str>> = vec![];
    let mut item_name_list = vec![];
    let mut location_id_list = vec![];
    let mut fulfillment_id_list = vec![];
    let mut item_image_list = vec![];
    let mut qty_list = vec![];
    let mut mrp_list = vec![];
    let mut unit_price_list = vec![];
    let mut tax_rate_list = vec![];
    let mut tax_amount_list = vec![];
    let mut discount_amount_list = vec![];
    let mut gross_amount_list = vec![];
    let mut available_qty_list = vec![];
    let mut item_req_list = vec![];
    let mut packaging_req_list = vec![];
    let discount_mapping = get_quote_item_value_mapping(
        &ondc_on_select_request.message.order.quote.breakup,
        &BreakupTitleType::Discount,
    );
    let tax_mapping = get_quote_item_value_mapping(
        &ondc_on_select_request.message.order.quote.breakup,
        &BreakupTitleType::Tax,
    );

    let item_breakup_mapping = get_quote_item_breakup_mapping(
        &ondc_on_select_request.message.order.quote.breakup,
        &BreakupTitleType::Item,
    );
    for item in &ondc_on_select_request.message.order.items {
        let key = get_ondc_seller_mapping_key(
            ondc_on_select_request
                .context
                .bpp_id
                .as_ref()
                .unwrap_or(&String::new()),
            &ondc_on_select_request.message.order.provider.id,
            &item.id,
        );
        let discount_amount = discount_mapping
            .get(&item.id)
            .cloned()
            .unwrap_or(BigDecimal::from(0));
        discount_amount_list.push(discount_amount);

        let tax_amount = tax_mapping
            .get(&item.id)
            .cloned()
            .unwrap_or(BigDecimal::from(0));

        if let Some(break_up) = item_breakup_mapping.get(&item.id) {
            unit_price_list.push(break_up.item.as_ref().map_or(BigDecimal::from(0), |a| {
                BigDecimal::from_str(&a.price.value).unwrap_or_else(|_| BigDecimal::from(0))
            }));
            available_qty_list.push(
                break_up
                    .quantity
                    .as_ref()
                    .map_or(BigDecimal::from(0), |a| BigDecimal::from(a.count)),
            );
            gross_amount_list.push(
                BigDecimal::from_str(&break_up.price.value).unwrap_or_else(|_| BigDecimal::from(0)),
            );
        } else {
            unit_price_list.push(BigDecimal::from(0));
            gross_amount_list.push(BigDecimal::from(0));
            available_qty_list.push(BigDecimal::from(0));
        }

        tax_amount_list.push(tax_amount);
        packaging_req_list.push(item.tags.as_ref().map(|tag| {
            get_tag_value_from_list(
                tag,
                ONDCTagType::BuyerTerms,
                &ONDCTagItemCode::PackagingsReq.to_string(),
            )
            .unwrap_or_default()
        }));
        item_req_list.push(item.tags.as_ref().map(|tag| {
            get_tag_value_from_list(
                tag,
                ONDCTagType::BuyerTerms,
                &ONDCTagItemCode::ItemReq.to_string(),
            )
            .unwrap_or_default()
        }));
        // if let Some(tags) = item.tags {
        //     packaging_req_list.push(get_tag_value_from_list(
        //         &tags,
        //         ONDCTagType::BuyerTerms,
        //         ONDCTagItemCode::PackagingsReq.to_string(),
        //     ));
        // } else
        // packagin_req_list.push(item.tags.as_ref().map(|a| {
        //     a[0].get_tag_value(&ONDCTagItemCode::PackagingsReq.to_string())
        //         .unwrap_or_default()
        // }));
        // item_req_list.push(item.tags.as_ref().map(|a| {
        //     a[0].get_tag_value(&ONDCTagItemCode::ItemReq.to_string())
        //         .unwrap_or_default()
        // }));
        if let Some(seller_item_obj) = product_map.get(&key) {
            item_code_list.push(seller_item_obj.item_code.as_deref());
            item_name_list.push(seller_item_obj.item_name.as_str());
            item_image_list.push(
                seller_item_obj
                    .images
                    .as_array()
                    .and_then(|images| images.first())
                    .and_then(|image| image.as_str())
                    .unwrap_or_default(),
            );
            mrp_list.push(seller_item_obj.mrp.clone());
            // unit_price_list.push(seller_item_obj.unit_price.clone());
            tax_rate_list.push(seller_item_obj.tax_rate.clone());
        } else {
            item_code_list.push(None);
            item_name_list.push("");
            item_image_list.push("");
            mrp_list.push(BigDecimal::from(0));

            tax_rate_list.push(BigDecimal::from(0));
        }
        item_id_list.push(item.id.as_str());

        location_id_list.push(serde_json::to_value(&item.location_ids)?); // Serialize to JSON
        fulfillment_id_list.push(serde_json::to_value(&item.fulfillment_ids)?);

        qty_list.push(BigDecimal::from(item.quantity.selected.count));
    }
    let query = sqlx::query!(
        r#"
        INSERT INTO buyer_commerce_data_line (id, commerce_data_id, item_id, item_name, item_code, item_image, 
            qty, location_ids, fulfillment_ids, tax_rate, mrp, unit_price, discount_amount, tax_value, gross_total,
            available_qty,item_req, packaging_req)
            SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::text[], $4::text[], $5::text[], $6::text[],
             $7::decimal[], $8::jsonb[], $9::jsonb[], $10::decimal[], $11::decimal[], $12::decimal[], $13::decimal[],
            $14::decimal[], $15::decimal[], $16::decimal[], $17::text[], $18::text[])
        ON CONFLICT (commerce_data_id, item_code) 
        DO UPDATE SET 
        fulfillment_ids = EXCLUDED.fulfillment_ids,
        unit_price = EXCLUDED.unit_price,
        discount_amount = EXCLUDED.discount_amount,
        tax_value = EXCLUDED.tax_value,
        gross_total = EXCLUDED.gross_total,
        available_qty = EXCLUDED.available_qty
        "#,
        &line_id_list[..] as &[Uuid],
        &order_id_list[..] as &[Uuid],
        &item_id_list[..] as &[&str],
        &item_name_list[..] as &[&str],
        &item_code_list[..] as &[Option<&str>],
        &item_image_list[..] as &[&str],
        &qty_list[..] as &[BigDecimal],
        &location_id_list as &[Value],
        &fulfillment_id_list as &[Value],
        &tax_rate_list[..] as &[BigDecimal],
        &mrp_list[..] as &[BigDecimal],
        &unit_price_list[..] as &[BigDecimal],
        &discount_amount_list[..] as &[BigDecimal],
        &tax_amount_list[..] as &[BigDecimal],
        &gross_amount_list[..] as &[BigDecimal],
        &available_qty_list[..] as &[BigDecimal],
        &item_req_list[..] as &[Option<&str>],
        &packaging_req_list[..] as &[Option<&str>],
    );

    transaction.execute(query).await.map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e)
            .context("A database failure occurred while saving RFQ to database request")
    })?;
    Ok(())
}

// let row: Option<AuthMechanismModel> = sqlx::query_as!(AuthMechanismModel,
//     r#"SELECT a.id as id, user_id, auth_identifier, secret, a.is_active as "is_active: Status", auth_scope as "auth_scope: AuthenticationScope", auth_context as "auth_context: AuthContextType", valid_upto from auth_mechanism
//     as a inner join user_account as b on a.user_id = b.id where (b.username = $1 OR b.mobile_no = $1 OR  b.email = $1)  AND auth_scope = $2 AND auth_context = $3"#,
//     username,
//     scope as &AuthenticationScope,
//     &auth_context as &AuthContextType
// )

#[tracing::instrument(name = "fetch buyer commerce data", skip(pool))]
async fn get_buyer_commerce_data(
    pool: &PgPool,
    transaction_id: &Uuid,
) -> Result<Option<BuyerCommerceDataModel>, anyhow::Error> {
    let record = sqlx::query_as!(
        BuyerCommerceDataModel,
        r#"
        
        SELECT id, urn, external_urn, record_type as "record_type:OrderType", 
           record_status as "record_status:CommerceStatusType",
           domain_category_code as "domain_category_code:CategoryDomain", 
           buyer_id, seller_id, buyer_name, seller_name, source as "source:DataSource", 
           created_on, updated_on, deleted_on, is_deleted, created_by, grand_total, 
           bpp_id, bpp_uri, bap_id, bap_uri, is_import, quote_ttl,
           currency_code as "currency_code?:CurrencyType", city_code,
           country_code as "country_code:CountryCode"
        FROM buyer_commerce_data where external_urn= $1;"#,
        transaction_id
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e).context(
            "A database failure occurred while saving fetching buyer_commerce data from database",
        )
    })?;

    Ok(record)
}

#[tracing::instrument(name = "fetch buyer commerce data line", skip(pool))]
async fn get_buyer_commerce_data_line(
    pool: &PgPool,
    order_id: &Uuid,
) -> Result<Vec<BuyerCommerceItemModel>, anyhow::Error> {
    let records = sqlx::query_as!(
        BuyerCommerceItemModel,
        r#"
        SELECT 
            id, 
            item_id, 
            commerce_data_id, 
            item_name, 
            item_code, 
            item_image, 
            qty, 
            packaging_req, 
            item_req,
            tax_rate, 
            tax_value, 
            unit_price, 
            gross_total, 
            available_qty, 
            discount_amount, 
            location_ids as "location_ids?: Json<Vec<String>>", 
            fulfillment_ids as "fulfillment_ids?: Json<Vec<String>>"
        FROM buyer_commerce_data_line 
        WHERE commerce_data_id = $1
        "#,
        order_id
    )
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e).context(
            "A database failure occurred while fetching buyer_commerce data line from database",
        )
    })?;

    Ok(records)
}

#[tracing::instrument(name = "fetch buyer commerce payments", skip(pool))]
async fn get_buyer_commerce_payments(
    pool: &PgPool,
    order_id: &Uuid,
) -> Result<Vec<BuyerCommercePaymentModel>, anyhow::Error> {
    let records = sqlx::query_as!(
        BuyerCommercePaymentModel,
        r#"
        SELECT 
            id, 
            collected_by as "collected_by?: ONDCNetworkType",
            payment_type as "payment_type!: PaymentType", 
            commerce_data_id
        FROM buyer_commerce_payment 
        WHERE commerce_data_id = $1
        "#,
        order_id
    )
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e).context(
            "A database failure occurred while fetching buyer_commerce data payment from database",
        )
    })?;

    Ok(records)
}

#[tracing::instrument(name = "fetch buyer commerce fulfillments", skip(pool))]
async fn get_buyer_commerce_fulfillments(
    pool: &PgPool,
    order_id: &Uuid,
) -> Result<Vec<BuyerCommerceFulfillmentModel>, anyhow::Error> {
    let records = sqlx::query_as!(
        BuyerCommerceFulfillmentModel,
        r#"
        SELECT 
            id,
            commerce_data_id,
            fulfillment_id,
            tat,
            fulfillment_type as "fulfillment_type: FulfillmentType",
            fulfillment_status as "fulfillment_status: CommerceFulfillmentStatusType",
            inco_terms as "inco_terms?: IncoTermType",
            place_of_delivery,
            provider_name,
            category as "category?: FulfillmentCategoryType",
            servicable_status as "servicable_status!: ServiceableType", 
            drop_off_data as "drop_off_data!:  Json<Option<DropOffDataModel>>",
            pickup_data as "pickup_data!:  Json<Option<PickUpDataModel>>",
            tracking
        FROM buyer_commerce_fulfillment_data 
        WHERE commerce_data_id = $1
        "#,
        order_id
    )
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e).context(
            "A database failure occurred while fetching buyer_commerce data fulfillments from database",
        )
    })?;

    Ok(records)
}

fn get_order_payment_from_model(
    payments: Vec<BuyerCommercePaymentModel>,
) -> Vec<BuyerCommercePayment> {
    let mut payment_obj = vec![];
    for payment in payments {
        payment_obj.push(BuyerCommercePayment {
            id: payment.id,
            collected_by: payment.collected_by,
            payment_type: payment.payment_type,
        })
    }
    payment_obj
}

fn get_order_items_from_model(items: Vec<BuyerCommerceItemModel>) -> Vec<BuyerCommerceItem> {
    let mut item_obj = vec![];
    for item in items {
        let buyer_term = if item.item_req.is_some() && item.packaging_req.is_some() {
            Some(BuyerTerm {
                item_req: item.item_req.unwrap(),
                packaging_req: item.packaging_req.unwrap(),
            })
        } else {
            None
        };
        let location_ids = item
            .location_ids
            .map(|json| json.0)
            .unwrap_or_else(Vec::new);
        let fulfillment_ids = item
            .fulfillment_ids
            .map(|json| json.0)
            .unwrap_or_else(Vec::new);
        item_obj.push(BuyerCommerceItem {
            id: item.id,
            item_id: item.item_id,
            item_name: item.item_name,
            item_code: item.item_code,
            item_image: item.item_image,
            qty: item.qty,
            buyer_terms: buyer_term,
            tax_rate: item.tax_rate,
            tax_value: item.tax_value,
            unit_price: item.unit_price,
            gross_total: item.gross_total,
            available_qty: item.available_qty,
            discount_amount: item.discount_amount,
            location_ids,
            fulfillment_ids,
        })
    }
    item_obj
}

fn get_drop_off_from_model(drop_off: DropOffDataModel) -> DropOffData {
    DropOffData {
        location: FulfillmentLocation {
            gps: drop_off.location.gps,
            area_code: drop_off.location.area_code,
            address: drop_off.location.address,
            city: drop_off.location.city,
            country: drop_off.location.country,
            state: drop_off.location.state,
        },
        contact: FulfillmentContact {
            mobile_no: drop_off.contact.mobile_no,
            email: drop_off.contact.email,
        },
    }
}

fn get_pick_up_from_model(pick_up: PickUpDataModel) -> PickUpData {
    PickUpData {
        location: FulfillmentLocation {
            gps: pick_up.location.gps,
            area_code: pick_up.location.area_code,
            address: pick_up.location.address,
            city: pick_up.location.city,
            country: pick_up.location.country,
            state: pick_up.location.state,
        },
        contact: FulfillmentContact {
            mobile_no: pick_up.contact.mobile_no,
            email: pick_up.contact.email,
        },
    }
}

fn get_order_fulfillment_from_model(
    fulfillments: Vec<BuyerCommerceFulfillmentModel>,
) -> Vec<BuyerCommerceFulfillment> {
    let mut fulfillment_obj = vec![];
    for fulfillment in fulfillments {
        let delivery_term =
            if fulfillment.inco_terms.is_some() && fulfillment.place_of_delivery.is_some() {
                Some(DeliveryTerm {
                    inco_terms: fulfillment.inco_terms.unwrap(),
                    place_of_delivery: fulfillment.place_of_delivery.unwrap(),
                })
            } else {
                None
            };
        fulfillment_obj.push(BuyerCommerceFulfillment {
            id: fulfillment.id,
            fulfillment_id: fulfillment.fulfillment_id,
            fulfillment_type: fulfillment.fulfillment_type,
            tat: fulfillment.tat,
            fulfillment_status: fulfillment.fulfillment_status,
            delivery_term,
            provider_name: fulfillment.provider_name,
            category: fulfillment.category,
            servicable_status: fulfillment.servicable_status,
            tracking: fulfillment.tracking,
            drop_off: fulfillment
                .drop_off_data
                .as_ref()
                .clone()
                .map(get_drop_off_from_model),
            pickup: fulfillment
                .pickup_data
                .as_ref()
                .clone()
                .map(get_pick_up_from_model),
        })
    }
    fulfillment_obj
}

#[tracing::instrument(name = "model to struct", skip())]
fn get_order_from_model(
    order: BuyerCommerceDataModel,
    lines: Vec<BuyerCommerceItemModel>,
    payments: Vec<BuyerCommercePaymentModel>,
    fulfillments: Vec<BuyerCommerceFulfillmentModel>,
) -> BuyerCommerce {
    BuyerCommerce {
        id: order.id,
        urn: order.urn,
        external_urn: order.external_urn,
        record_type: order.record_type,
        record_status: order.record_status,
        domain_category_code: order.domain_category_code,
        seller: BuyerCommerceSeller {
            id: order.seller_id,
            name: order.seller_name,
        },
        source: order.source,
        created_on: order.created_on,
        updated_on: order.updated_on,
        created_by: order.created_by,
        grand_total: order.grand_total,
        bap: BasicNetWorkData {
            id: order.bap_id,
            uri: order.bap_uri,
        },
        bpp: BasicNetWorkData {
            id: order.bpp_id,
            uri: order.bpp_uri,
        },
        is_import: order.is_import,
        quote_ttl: order.quote_ttl,
        city_code: order.city_code,
        country_code: order.country_code,
        payments: get_order_payment_from_model(payments),
        items: get_order_items_from_model(lines),
        fulfillments: get_order_fulfillment_from_model(fulfillments),
    }
}

#[tracing::instrument(name = "fetch order", skip(pool))]
pub async fn fetch_order_by_id(
    pool: &PgPool,
    transaction_id: &Uuid,
) -> Result<Option<BuyerCommerce>, anyhow::Error> {
    if let Some(order_data) = get_buyer_commerce_data(pool, transaction_id).await? {
        let lines = get_buyer_commerce_data_line(pool, &order_data.id).await?;
        let payments = get_buyer_commerce_payments(pool, &order_data.id).await?;
        let fulfillmets = get_buyer_commerce_fulfillments(pool, &order_data.id).await?;
        Ok(Some(get_order_from_model(
            order_data,
            lines,
            payments,
            fulfillmets,
        )))
    } else {
        Ok(None)
    }
}

#[tracing::instrument(name = "delete payment on on_init", skip(transaction))]
async fn delete_payment_in_on_init(
    transaction: &mut Transaction<'_, Postgres>,
    transaction_id: &Uuid,
) -> Result<Uuid, anyhow::Error> {
    let query = sqlx::query!(
        r#"
        DELETE FROM buyer_commerce_payment USING buyer_commerce_data 
        WHERE buyer_commerce_payment.commerce_data_id = buyer_commerce_data.id 
        AND buyer_commerce_data.external_urn = $1 
        RETURNING buyer_commerce_data.id AS bc_id;
        "#,
        transaction_id // Pass the parameter here
    );

    let result = query.fetch_one(&mut **transaction).await.map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e)
            .context("A database failure occurred while saving RFQ to database request")
    })?;
    Ok(result.bc_id)
}

#[tracing::instrument(name = "save payment on on_init", skip(transaction))]
pub async fn initialize_payment_on_init(
    transaction: &mut Transaction<'_, Postgres>,
    commerce_id: &Uuid,
    payments: &Vec<ONDCOnInitPayment>,
) -> Result<(), anyhow::Error> {
    let mut id_list = vec![];
    let mut commerce_data_id_list = vec![];
    let mut collected_by_list = vec![];
    let mut payment_type_list = vec![];
    let mut buyer_fee_type_list = vec![];
    let mut buyer_fee_amount_list = vec![];
    let mut settlement_window_list = vec![];
    let mut withholding_amount_list = vec![];
    let mut seller_payment_uri_list = vec![];
    let mut settlement_basis_list = vec![];
    let mut seller_payment_ttl = vec![];
    let mut seller_payment_dsa_list = vec![];
    let mut seller_payment_signature_list = vec![];
    let mut settlement_detail_list = vec![];
    for payment in payments {
        id_list.push(Uuid::new_v4());
        commerce_data_id_list.push(*commerce_id);
        collected_by_list.push(payment.collected_by.clone());
        payment_type_list.push(payment.r#type.get_payment());
        buyer_fee_type_list.push(&payment.buyer_app_finder_fee_type);
        buyer_fee_amount_list
            .push(BigDecimal::from_str(&payment.buyer_app_finder_fee_amount).unwrap());
        settlement_window_list.push(payment.settlement_window.as_str());
        withholding_amount_list.push(BigDecimal::from_str(&payment.withholding_amount).unwrap());
        seller_payment_uri_list.push(payment.uri.as_deref());
        settlement_basis_list.push(&payment.settlement_basis);
        seller_payment_ttl.push(payment.tags.as_ref().map(|tag| {
            get_tag_value_from_list(
                tag,
                ONDCTagType::BPPPayment,
                &ONDCTagItemCode::Ttl.to_string(),
            )
            .unwrap_or_default()
        }));
        seller_payment_dsa_list.push(payment.tags.as_ref().map(|tag| {
            get_tag_value_from_list(
                tag,
                ONDCTagType::BPPPayment,
                &ONDCTagItemCode::Dsa.to_string(),
            )
            .unwrap_or_default()
        }));
        seller_payment_signature_list.push(payment.tags.as_ref().map(|tag| {
            get_tag_value_from_list(
                tag,
                ONDCTagType::BPPPayment,
                &ONDCTagItemCode::Signature.to_string(),
            )
            .unwrap_or_default()
        }));
        let settlement_details: Option<Vec<PaymentSettlementDetailModel>> = payment
            .settlement_details
            .as_ref() // Borrow the Option<Vec<ONDCPaymentSettlementDetail>>
            .map(|details| {
                details
                    .iter()
                    .map(|e| e.to_payment_settlement_detail())
                    .collect::<Vec<PaymentSettlementDetailModel>>()
            });
        settlement_detail_list.push(serde_json::to_value(settlement_details).unwrap());
    }
    let query = sqlx::query!(
        r#"
        INSERT INTO buyer_commerce_payment(id, commerce_data_id, collected_by, payment_type, buyer_fee_type,
             buyer_fee_amount, settlement_window, withholding_amount, seller_payment_uri, settlement_basis,
             seller_payment_ttl, seller_payment_dsa, seller_payment_signature, settlement_details)
            SELECT * FROM UNNEST($1::uuid[], $2::uuid[], $3::ondc_network_participant_type[],
            $4::payment_type[], $5::ondc_np_fee_type[], $6::decimal[], $7::text[], $8::decimal[],
            $9::text[], $10::settlement_basis_type[], $11::text[], $12::text[],  $13::text[],$14::jsonb[])
        "#,
        &id_list[..] as &[Uuid],
        &commerce_data_id_list[..] as &[Uuid],
        &collected_by_list[..] as &[ONDCNetworkType],
        &payment_type_list[..] as &[PaymentType],
        &buyer_fee_type_list[..] as &[&FeeType],
        &buyer_fee_amount_list[..] as &[BigDecimal],
        &settlement_window_list[..] as &[&str],
        &withholding_amount_list[..] as &[BigDecimal],
        &seller_payment_uri_list[..] as &[Option<&str>],
        &settlement_basis_list[..] as &[&SettlementBasis],
        &seller_payment_ttl[..] as &[Option<&str>],
        &seller_payment_dsa_list[..] as &[Option<&str>],
        &seller_payment_signature_list[..] as &[Option<&str>],
        &settlement_detail_list[..] as &[Value],
    );

    transaction.execute(query).await.map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e)
            .context("A database failure occurred while saving RFQ to database request")
    })?;
    Ok(())
}

#[tracing::instrument(name = "update buyer commerce data on on_init", skip())]
fn convert_ondc_billing_to_model_billing(billing: &ONDCBilling) -> OrderBillingModel {
    OrderBillingModel {
        name: billing.name.clone(),
        address: billing.address.clone(),
        state: billing.state.name.clone(),
        city: billing.city.name.clone(),
        tax_id: billing.tax_id.clone(),
        email: billing.email.clone(),
        phone: billing.phone.clone(),
    }
}

pub fn get_bpp_term_model_from_tag(tags: &[ONDCTag]) -> BPPTermsModel {
    let max_liability = get_tag_value_from_list(
        tags,
        ONDCTagType::BppTerms,
        &ONDCTagItemCode::MaxLiability.to_string(),
    )
    .unwrap_or_default()
    .to_owned();

    let max_liability_cap = get_tag_value_from_list(
        tags,
        ONDCTagType::BppTerms,
        &ONDCTagItemCode::MaxLiabilityCap.to_string(),
    )
    .unwrap_or_default()
    .to_owned();

    let mandatory_arbitration = get_tag_value_from_list(
        tags,
        ONDCTagType::BppTerms,
        &ONDCTagItemCode::MandatoryArbitration.to_string(),
    )
    .unwrap_or_default()
        == "false";

    let court_jurisdiction = get_tag_value_from_list(
        tags,
        ONDCTagType::BppTerms,
        &ONDCTagItemCode::CourtJurisdiction.to_string(),
    )
    .unwrap_or_default()
    .to_owned();
    let delay_interest = get_tag_value_from_list(
        tags,
        ONDCTagType::BppTerms,
        &ONDCTagItemCode::DelayInterest.to_string(),
    )
    .unwrap_or_default()
    .to_owned();
    BPPTermsModel {
        max_liability,
        max_liability_cap,
        mandatory_arbitration,
        court_jurisdiction,
        delay_interest,
    }
}

pub fn get_cancel_term_model_from_ondc_cancel_term(
    cancellation_terms: &Vec<ONDCOrderCancellationTerm>,
) -> Vec<OrderCancellationTermModel> {
    let mut cancelletion_list = vec![];
    for cancellation_term in cancellation_terms {
        cancelletion_list.push(OrderCancellationTermModel {
            fulfillment_state: cancellation_term
                .fulfillment_state
                .descriptor
                .code
                .get_fulfillment_state(),
            reason_required: cancellation_term.reason_required,
            cancellation_fee: OrderCancellationFeeModel {
                r#type: cancellation_term.cancellation_fee.get_type(),
                val: cancellation_term.cancellation_fee.get_amount(),
            },
        })
    }
    cancelletion_list
}

#[tracing::instrument(name = "update buyer commerce data on on_init", skip(transaction))]
async fn update_buyer_commerce_in_on_init(
    transaction: &mut Transaction<'_, Postgres>,
    on_init_request: &ONDCOnInitRequest,
) -> Result<(), anyhow::Error> {
    let billing = convert_ondc_billing_to_model_billing(&on_init_request.message.order.billing);
    let bpp_terms = get_bpp_term_model_from_tag(&on_init_request.message.order.tags);
    let cancellation_terms = get_cancel_term_model_from_ondc_cancel_term(
        &on_init_request.message.order.cancellation_terms,
    );

    let query = sqlx::query!(
        r#"
        UPDATE buyer_commerce_data SET billing=$1, bpp_terms=$2, record_status=$3, cancellation_terms=$4 WHERE external_urn=$5
        "#,
        serde_json::to_value(billing).unwrap(),
        serde_json::to_value(bpp_terms).unwrap(),
        CommerceStatusType::Initialized as CommerceStatusType,
        serde_json::to_value(cancellation_terms).unwrap(),
        on_init_request.context.transaction_id,
    );

    transaction.execute(query).await.map_err(|e| {
        tracing::error!("Failed to execute query: {:?}", e);
        anyhow::Error::new(e)
            .context("A database failure occurred while saving on_init buyer commerce to database")
    })?;
    Ok(())
}

#[tracing::instrument(name = "save order on on_init", skip(pool))]
pub async fn initialize_order_on_init(
    pool: &PgPool,
    on_init_request: &ONDCOnInitRequest,
) -> Result<(), anyhow::Error> {
    let mut transaction = pool
        .begin()
        .await
        .context("Failed to acquire a Postgres connection from the pool")?;
    let commerce_id =
        delete_payment_in_on_init(&mut transaction, &on_init_request.context.transaction_id)
            .await?;
    initialize_payment_on_init(
        &mut transaction,
        &commerce_id,
        &on_init_request.message.order.payments,
    )
    .await?;
    update_buyer_commerce_in_on_init(&mut transaction, on_init_request).await?;
    transaction
        .commit()
        .await
        .context("Failed to commit SQL transaction to update order on init")?;
    Ok(())
}
